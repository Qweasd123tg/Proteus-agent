use std::{
    fs::OpenOptions as StdOpenOptions,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use proteus_contracts::domain::{ExecutionId, SessionId, ThreadId, TurnId, new_record_id};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use tokio::{fs::OpenOptions, io::AsyncWriteExt};

use super::{
    projection::{JournalProjection, JournalValidationState},
    types::{JOURNAL_SCHEMA_VERSION, JournalEntry, JournalKind, JournalRecord},
};

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

#[derive(Debug, Clone, Default)]
pub(crate) struct JournalWriterState {
    initialized: bool,
    next_seq: u64,
    validation: JournalValidationState,
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

pub(crate) async fn append_record(
    session_dir: &Path,
    session_id: SessionId,
    attribution: JournalRecordAttribution,
    entry: JournalEntry,
    blob_threshold_bytes: usize,
    state: &mut JournalWriterState,
) -> Result<JournalRecord> {
    repair_unterminated_tail(&journal_path(session_dir))?;
    initialize_writer_state(session_dir, session_id, state)?;

    let kind = entry.kind();
    let mut payload = entry.payload_value()?;
    redact_sensitive_values(&mut payload);
    let payload_bytes = serde_json::to_vec(&payload)?;
    if payload_bytes.len() > MAX_PAYLOAD_BYTES {
        bail!(
            "journal payload is {} bytes, maximum is {} bytes",
            payload_bytes.len(),
            MAX_PAYLOAD_BYTES
        );
    }
    let entry = JournalEntry::from_kind_and_payload(kind, payload.clone())
        .context("redacted journal payload no longer matches its canonical DTO")?;
    let record = JournalRecord {
        schema_version: JOURNAL_SCHEMA_VERSION,
        record_id: new_record_id(),
        session_seq: state.next_seq,
        timestamp_ms: unix_timestamp_ms(),
        session_id,
        execution_id: attribution.execution_id,
        thread_id: attribution.thread_id,
        turn_id: attribution.turn_id,
        entry,
    };
    let mut next_validation = state.validation.clone();
    next_validation.apply(&record)?;

    let stored_payload = if payload_bytes.len() >= blob_threshold_bytes {
        write_blob(session_dir, &payload_bytes).await?
    } else {
        StoredPayload::Inline { value: payload }
    };
    let stored = StoredJournalRecord {
        schema_version: record.schema_version,
        record_id: record.record_id,
        session_seq: record.session_seq,
        timestamp_ms: record.timestamp_ms,
        session_id: record.session_id,
        execution_id: record.execution_id,
        thread_id: record.thread_id,
        turn_id: record.turn_id,
        kind,
        payload: stored_payload,
    };
    let mut line = serde_json::to_vec(&stored)?;
    line.push(b'\n');
    let path = journal_path(session_dir);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .with_context(|| format!("failed to open {}", path.display()))?;
    file.write_all(&line)
        .await
        .with_context(|| format!("failed to append {}", path.display()))?;
    file.flush().await?;
    file.sync_data().await?;

    state.next_seq = state.next_seq.saturating_add(1);
    state.validation = next_validation;
    Ok(record)
}

pub(crate) fn initialize_writer_state(
    session_dir: &Path,
    session_id: SessionId,
    state: &mut JournalWriterState,
) -> Result<()> {
    if state.initialized {
        return Ok(());
    }
    repair_unterminated_tail(&journal_path(session_dir))?;
    let records = load_records(session_dir, session_id)?;
    let projection = JournalProjection::build(session_id, records.clone())?;
    let mut validation = JournalValidationState::default();
    for record in &records {
        validation.apply(record)?;
    }
    state.next_seq = records
        .last()
        .map(|record| record.session_seq.saturating_add(1))
        .unwrap_or(1);
    debug_assert_eq!(validation.history_revision(), projection.history_revision);
    state.validation = validation;
    state.initialized = true;
    Ok(())
}

impl JournalWriterState {
    pub(crate) fn history_revision(&self) -> u64 {
        self.validation.history_revision()
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

async fn write_blob(session_dir: &Path, bytes: &[u8]) -> Result<StoredPayload> {
    let sha256 = sha256_hex(bytes);
    let relative_path = PathBuf::from(BLOBS_DIR).join(format!("{sha256}.json"));
    let path = session_dir.join(&relative_path);
    tokio::fs::create_dir_all(session_dir.join(BLOBS_DIR)).await?;
    if tokio::fs::try_exists(&path).await? {
        let existing = tokio::fs::read(&path).await?;
        if existing != bytes {
            bail!("content-addressed blob collision at {}", path.display());
        }
    } else {
        let tmp_path = session_dir
            .join(BLOBS_DIR)
            .join(format!(".{sha256}.tmp.{}", new_record_id()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)
            .await
            .with_context(|| format!("failed to create {}", tmp_path.display()))?;
        file.write_all(bytes).await?;
        file.flush().await?;
        file.sync_data().await?;
        match tokio::fs::rename(&tmp_path, &path).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let existing = tokio::fs::read(&path).await?;
                if existing != bytes {
                    bail!("content-addressed blob collision at {}", path.display());
                }
                let _ = tokio::fs::remove_file(&tmp_path).await;
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
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

fn redact_sensitive_values(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, nested) in map {
                if is_sensitive_key(key) {
                    *nested = serde_json::Value::String("[REDACTED]".to_owned());
                } else {
                    redact_sensitive_values(nested);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                redact_sensitive_values(nested);
            }
        }
        _ => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().replace('-', "_").as_str(),
        "authorization"
            | "api_key"
            | "access_token"
            | "refresh_token"
            | "session_token"
            | "password"
            | "secret"
            | "cookie"
            | "set_cookie"
    )
}
