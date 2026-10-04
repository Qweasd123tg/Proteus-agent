use std::{
    fs::OpenOptions as StdOpenOptions,
    io::{ErrorKind, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use proteus_contracts::domain::{ExecutionId, SessionId, ThreadId, TurnId, new_record_id};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};

use super::{
    projection::{JournalProjection, JournalValidationState},
    types::{JOURNAL_SCHEMA_VERSION, JournalEntry, JournalKind, JournalRecord},
};

mod append;
mod ownership;
pub(crate) use append::append_record;
mod recovery;
mod redaction;

use ownership::SessionWriteOwnership;
use recovery::{AppendRollback, restore_committed_offset};
pub(super) use redaction::contains_redacted_sensitive_value;
use redaction::redact_sensitive_values;
pub(crate) use redaction::redacted_history;

pub const JOURNAL_FILE: &str = "journal.jsonl";
const BLOBS_DIR: &str = "blobs";
pub const DEFAULT_BLOB_THRESHOLD_BYTES: usize = 256 * 1024;
const MAX_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredJournalRecord {
    schema_version: u32,
    record_id: proteus_contracts::domain::RecordId,
    session_seq: u64,
    timestamp_ms: i64,
    session_id: SessionId,
    execution_id: Option<ExecutionId>,
    thread_id: Option<ThreadId>,
    turn_id: Option<TurnId>,
    kind: JournalKind,
    payload: StoredPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "storage", rename_all = "snake_case", deny_unknown_fields)]
enum StoredPayload {
    Inline {
        value: serde_json::Value,
    },
    Blob {
        sha256: String,
        bytes: u64,
        relative_path: PathBuf,
    },
}

#[derive(Debug, Default)]
pub(crate) struct JournalWriterState {
    initialized: bool,
    usage: super::UsageProjection,
    next_seq: u64,
    validation: JournalValidationState,
    committed_offset: u64,
    _ownership: Option<SessionWriteOwnership>,
    #[cfg(test)]
    initial_recovery_scans: usize,
    #[cfg(test)]
    append_pause: Option<append::AppendPause>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct JournalRecordAttribution {
    pub execution_id: Option<ExecutionId>,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
}

impl JournalRecordAttribution {
    pub fn chat(thread_id: ThreadId, turn_id: Option<TurnId>) -> Self {
        Self {
            execution_id: None,
            thread_id: Some(thread_id),
            turn_id,
        }
    }

    pub fn execution(
        execution_id: ExecutionId,
        thread_id: Option<ThreadId>,
        turn_id: Option<TurnId>,
    ) -> Self {
        Self {
            execution_id: Some(execution_id),
            thread_id,
            turn_id,
        }
    }
}

pub(crate) fn initialize_writer_state(
    session_dir: &Path,
    session_id: SessionId,
    state: &mut JournalWriterState,
) -> Result<()> {
    if state.initialized {
        return Ok(());
    }
    let ownership = SessionWriteOwnership::acquire(session_dir, session_id)?;
    let path = journal_path(session_dir);
    repair_unterminated_tail(&path)?;
    #[cfg(test)]
    {
        state.initial_recovery_scans += 1;
    }
    let records = load_records(session_dir, session_id)?;
    let projection = JournalProjection::build(session_id, records.clone())?;
    let mut validation = JournalValidationState::default();
    let mut usage = super::UsageProjection::default();
    for record in &records {
        validation.apply(record)?;
        usage.apply(record);
    }
    state.next_seq = records
        .last()
        .map(|record| record.session_seq.saturating_add(1))
        .unwrap_or(1);
    debug_assert_eq!(validation.history_revision(), projection.history_revision);
    state.validation = validation;
    state.usage = usage;
    state.committed_offset = journal_len(&path)?;
    state._ownership = Some(ownership);
    state.initialized = true;
    Ok(())
}

impl JournalWriterState {
    pub(crate) fn usage_snapshot(
        &self,
        session_id: SessionId,
    ) -> Option<crate::domain::SessionUsageSnapshot> {
        self.initialized.then(|| self.usage.snapshot(session_id))
    }

    pub(crate) fn history_revision(&self) -> u64 {
        self.validation.history_revision()
    }

    #[cfg(test)]
    pub(crate) fn initial_recovery_scans(&self) -> usize {
        self.initial_recovery_scans
    }
}

pub(crate) fn load_records(
    session_dir: &Path,
    expected_session_id: SessionId,
) -> Result<Vec<JournalRecord>> {
    let path = journal_path(session_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let complete_len = complete_jsonl_len(&bytes);
    let complete = &bytes[..complete_len];
    let mut records = Vec::new();
    if complete.is_empty() {
        return Ok(records);
    }
    let body = &complete[..complete.len() - 1];
    for (index, line) in body.split(|byte| *byte == b'\n').enumerate() {
        if line.is_empty() {
            bail!(
                "journal {} contains an empty line at {}",
                path.display(),
                index + 1
            );
        }
        let stored: StoredJournalRecord = serde_json::from_slice(line).with_context(|| {
            format!(
                "failed to parse journal {} line {}",
                path.display(),
                index + 1
            )
        })?;
        if stored.schema_version != JOURNAL_SCHEMA_VERSION {
            bail!(
                "unsupported journal schema_version {} in {} line {}; expected {}",
                stored.schema_version,
                path.display(),
                index + 1,
                JOURNAL_SCHEMA_VERSION
            );
        }
        if stored.session_id != expected_session_id {
            bail!(
                "journal {} line {} belongs to session {}, expected {}",
                path.display(),
                index + 1,
                stored.session_id,
                expected_session_id
            );
        }
        let payload = hydrate_payload(session_dir, &stored.payload).with_context(|| {
            format!(
                "failed to hydrate journal {} line {}",
                path.display(),
                index + 1
            )
        })?;
        let entry =
            JournalEntry::from_kind_and_payload(stored.kind, payload).with_context(|| {
                format!(
                    "invalid journal payload in {} line {}",
                    path.display(),
                    index + 1
                )
            })?;
        records.push(JournalRecord {
            schema_version: stored.schema_version,
            record_id: stored.record_id,
            session_seq: stored.session_seq,
            timestamp_ms: stored.timestamp_ms,
            session_id: stored.session_id,
            execution_id: stored.execution_id,
            thread_id: stored.thread_id,
            turn_id: stored.turn_id,
            entry,
        });
    }
    Ok(records)
}

pub(crate) fn journal_path(session_dir: &Path) -> PathBuf {
    session_dir.join(JOURNAL_FILE)
}

fn complete_jsonl_len(bytes: &[u8]) -> usize {
    if bytes.ends_with(b"\n") {
        bytes.len()
    } else {
        bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(0)
    }
}

fn repair_unterminated_tail(path: &Path) -> Result<()> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let complete_len = complete_jsonl_len(&bytes);
    if complete_len == bytes.len() {
        return Ok(());
    }
    let file = StdOpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open {} for tail recovery", path.display()))?;
    file.set_len(complete_len as u64)
        .with_context(|| format!("failed to truncate interrupted tail in {}", path.display()))?;
    file.sync_data()?;
    Ok(())
}

fn journal_len(path: &Path) -> Result<u64> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

fn write_blob(session_dir: &Path, bytes: &[u8]) -> Result<StoredPayload> {
    let sha256 = sha256_hex(bytes);
    let relative_path = PathBuf::from(BLOBS_DIR).join(format!("{sha256}.json"));
    let path = session_dir.join(&relative_path);
    std::fs::create_dir_all(session_dir.join(BLOBS_DIR))?;
    if path.try_exists()? {
        let existing = std::fs::read(&path)?;
        if existing != bytes {
            bail!("content-addressed blob collision at {}", path.display());
        }
    } else {
        let tmp_path = session_dir
            .join(BLOBS_DIR)
            .join(format!(".{sha256}.tmp.{}", new_record_id()));
        let mut file = StdOpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)
            .with_context(|| format!("failed to create {}", tmp_path.display()))?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_data()?;
        match std::fs::rename(&tmp_path, &path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let existing = std::fs::read(&path)?;
                if existing != bytes {
                    bail!("content-addressed blob collision at {}", path.display());
                }
                let _ = std::fs::remove_file(&tmp_path);
            }
            Err(error) => {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(error)
                    .with_context(|| format!("failed to install blob {}", path.display()));
            }
        }
    }
    Ok(StoredPayload::Blob {
        sha256,
        bytes: bytes.len() as u64,
        relative_path,
    })
}

fn hydrate_payload(session_dir: &Path, payload: &StoredPayload) -> Result<serde_json::Value> {
    match payload {
        StoredPayload::Inline { value } => {
            let bytes = serde_json::to_vec(value)?.len();
            if bytes > MAX_PAYLOAD_BYTES {
                bail!(
                    "inline journal payload is {bytes} bytes, maximum is {MAX_PAYLOAD_BYTES} bytes"
                );
            }
            Ok(value.clone())
        }
        StoredPayload::Blob {
            sha256,
            bytes,
            relative_path,
        } => {
            validate_blob_reference(sha256, relative_path)?;
            if *bytes > MAX_PAYLOAD_BYTES as u64 {
                bail!("journal blob declares {bytes} bytes, maximum is {MAX_PAYLOAD_BYTES} bytes");
            }
            let path = session_dir.join(relative_path);
            let actual_bytes = std::fs::metadata(&path)
                .with_context(|| format!("failed to inspect journal blob {}", path.display()))?
                .len();
            if actual_bytes != *bytes {
                bail!(
                    "journal blob {} size mismatch: expected {}, found {}",
                    path.display(),
                    bytes,
                    actual_bytes
                );
            }
            let content = std::fs::read(&path)
                .with_context(|| format!("failed to read journal blob {}", path.display()))?;
            let actual = sha256_hex(&content);
            if actual != *sha256 {
                bail!(
                    "journal blob {} hash mismatch: expected {}, found {}",
                    path.display(),
                    sha256,
                    actual
                );
            }
            serde_json::from_slice(&content)
                .with_context(|| format!("journal blob {} is not JSON", path.display()))
        }
    }
}

fn validate_blob_reference(sha256: &str, relative_path: &Path) -> Result<()> {
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid journal blob sha256 '{sha256}'");
    }
    let components = relative_path.components().collect::<Vec<_>>();
    let expected_name = format!("{}.json", sha256.to_ascii_lowercase());
    match components.as_slice() {
        [Component::Normal(dir), Component::Normal(file)]
            if *dir == std::ffi::OsStr::new(BLOBS_DIR)
                && *file == std::ffi::OsStr::new(&expected_name) =>
        {
            Ok(())
        }
        _ => Err(anyhow!(
            "journal blob path must be blobs/<sha256>.json, found {}",
            relative_path.display()
        )),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn unix_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(i64::MAX))
        .unwrap_or(0)
}
