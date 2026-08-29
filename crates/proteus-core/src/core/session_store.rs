use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result, anyhow};
use proteus_contracts::app_protocol::AppSessionSummary;
use tokio::sync::Mutex;

use crate::{
    contracts::ExecutionAttribution,
    core::session_journal::{
        DEFAULT_BLOB_THRESHOLD_BYTES, HistoryMutated, HistoryMutationKind, JournalEntry,
        JournalProjection, JournalRecord, JournalRecordAttribution, JournalWriterState,
        append_record, initialize_writer_state, journal_path, load_records,
    },
    domain::{ExecutionId, HistoryCompactionReport, SessionId, ThreadId, TurnId},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

mod identity;
mod workspace_dir;

use identity::{
    SessionDirectoryKind, ensure_writable_identity, resolve_session_identity,
    short_session_directory_name, validate_new_session_target,
};
use workspace_dir::workspace_path_from_session_dir;
pub use workspace_dir::{decode_workspace_path, encode_workspace_path};

#[derive(Debug, Clone)]
pub struct SessionStore {
    session_dir: PathBuf,
    session_id: SessionId,
    directory_kind: SessionDirectoryKind,
    writer: Arc<Mutex<JournalWriterState>>,
    blob_threshold_bytes: usize,
}

impl SessionStore {
    pub fn new(config_dir: &Path, cwd: &Path, session_id: SessionId) -> Result<Self> {
        let workspace = encode_workspace_path(cwd)?;
        let session_dir = config_dir
            .join("sessions")
            .join(workspace)
            .join(short_session_directory_name(session_id));
        validate_new_session_target(&session_dir, session_id)?;
        let writer = writer_for_session_dir(&session_dir);
        Ok(Self {
            session_dir,
            session_id,
            directory_kind: SessionDirectoryKind::ShortNumeric,
            writer,
            blob_threshold_bytes: DEFAULT_BLOB_THRESHOLD_BYTES,
        })
    }

    pub fn open(session_dir: PathBuf) -> Result<Self> {
        let identity = resolve_session_identity(&session_dir)?;
        workspace_path_from_session_dir(&session_dir)?;
        let writer = writer_for_session_dir(&session_dir);
        Ok(Self {
            session_dir,
            session_id: identity.session_id,
            directory_kind: identity.directory_kind,
            writer,
            blob_threshold_bytes: DEFAULT_BLOB_THRESHOLD_BYTES,
        })
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn workspace_path(&self) -> Result<PathBuf> {
        workspace_path_from_session_dir(&self.session_dir)
    }

    pub fn journal_path(&self) -> PathBuf {
        journal_path(&self.session_dir)
    }

    async fn materialize_for_write(&self) -> Result<()> {
        let parent = self.session_dir.parent().ok_or_else(|| {
            anyhow!(
                "session directory has no parent: {}",
                self.session_dir.display()
            )
        })?;
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create session parent {}", parent.display()))?;
        let created = match tokio::fs::create_dir(&self.session_dir).await {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to create session dir {}",
                        self.session_dir.display()
                    )
                });
            }
        };
        let workspace_path = self.workspace_path()?;
        ensure_writable_identity(
            &self.session_dir,
            self.session_id,
            self.directory_kind,
            &workspace_path,
            created,
        )
        .await
    }

    pub fn load_messages(&self) -> Result<Vec<CanonicalMessage>> {
        Ok(self.load_projection()?.history)
    }

    pub fn load_records(&self) -> Result<Vec<JournalRecord>> {
        load_records(&self.session_dir, self.session_id)
    }

    pub fn load_projection(&self) -> Result<JournalProjection> {
        JournalProjection::build(self.session_id, self.load_records()?)
    }

    pub async fn append_history(
        &self,
        thread_id: ThreadId,
        turn_id: Option<TurnId>,
        messages: &[CanonicalMessage],
    ) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        let mut writer = self.writer.lock().await;
        self.materialize_for_write().await?;
        initialize_writer_state(&self.session_dir, self.session_id, &mut writer)?;
        let previous_revision = writer.history_revision();
        append_record(
            &self.session_dir,
            self.session_id,
            JournalRecordAttribution::chat(thread_id, turn_id),
            JournalEntry::HistoryMutated(HistoryMutated {
                previous_revision,
                new_revision: previous_revision.saturating_add(1),
                mutation: HistoryMutationKind::Append,
                messages: messages.to_vec(),
                compaction: None,
            }),
            self.blob_threshold_bytes,
            &mut writer,
        )
        .await?;
        Ok(())
    }

    pub async fn replace_history(
        &self,
        thread_id: ThreadId,
        turn_id: Option<TurnId>,
        messages: &[CanonicalMessage],
        compaction: Option<HistoryCompactionReport>,
    ) -> Result<()> {
        let mut writer = self.writer.lock().await;
        self.materialize_for_write().await?;
        initialize_writer_state(&self.session_dir, self.session_id, &mut writer)?;
        let previous_revision = writer.history_revision();
        append_record(
            &self.session_dir,
            self.session_id,
            JournalRecordAttribution::chat(thread_id, turn_id),
            JournalEntry::HistoryMutated(HistoryMutated {
                previous_revision,
                new_revision: previous_revision.saturating_add(1),
                mutation: HistoryMutationKind::Replace,
                messages: messages.to_vec(),
                compaction,
            }),
            self.blob_threshold_bytes,
            &mut writer,
        )
        .await?;
        Ok(())
    }

    pub async fn append_journal_entry(
        &self,
        thread_id: ThreadId,
        turn_id: Option<TurnId>,
        entry: JournalEntry,
    ) -> Result<JournalRecord> {
        let mut writer = self.writer.lock().await;
        self.materialize_for_write().await?;
        append_record(
            &self.session_dir,
            self.session_id,
            JournalRecordAttribution::chat(thread_id, turn_id),
            entry,
            self.blob_threshold_bytes,
            &mut writer,
        )
        .await
    }

    pub async fn append_execution_journal_entry(
        &self,
        attribution: ExecutionAttribution,
        entry: JournalEntry,
    ) -> Result<JournalRecord> {
        let (thread_id, turn_id) = match attribution.agent {
            Some(agent) => {
                if agent.session_id != self.session_id {
                    anyhow::bail!(
                        "execution attribution belongs to session {}, recorder store belongs to {}",
                        agent.session_id,
                        self.session_id
                    );
                }
                (Some(agent.thread_id), Some(agent.turn_id))
            }
            None => (None, None),
        };
        self.append_execution_fact(attribution.execution_id, thread_id, turn_id, entry)
            .await
    }

    async fn append_execution_fact(
        &self,
        execution_id: ExecutionId,
        thread_id: Option<ThreadId>,
        turn_id: Option<TurnId>,
        entry: JournalEntry,
    ) -> Result<JournalRecord> {
        let mut writer = self.writer.lock().await;
        self.materialize_for_write().await?;
        append_record(
            &self.session_dir,
            self.session_id,
            JournalRecordAttribution::execution(execution_id, thread_id, turn_id),
            entry,
            self.blob_threshold_bytes,
            &mut writer,
        )
        .await
    }

    pub async fn clear_history(&self, thread_id: ThreadId) -> Result<()> {
        self.replace_history(thread_id, None, &[], None).await
    }
}

pub fn list_session_summaries(config_root: &Path) -> Result<Vec<AppSessionSummary>> {
    let sessions_root = config_root.join("sessions");
    let workspace_dirs = match std::fs::read_dir(&sessions_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", sessions_root.display()));
        }
    };

    let mut summaries = Vec::new();
    for workspace_entry in workspace_dirs {
        let workspace_entry = workspace_entry?;
        if !workspace_entry.file_type()?.is_dir() {
            continue;
        }

        for session_entry in std::fs::read_dir(workspace_entry.path())? {
            let session_entry = session_entry?;
            if !session_entry.file_type()?.is_dir() {
                continue;
            }

            let session_dir = session_entry.path();
            let summary = session_summary_from_dir(session_dir)?;
            if summary.message_count > 0 {
                summaries.push(summary);
            }
        }
    }

    summaries.sort_by(|left, right| {
        right
            .updated_at_ms
            .cmp(&left.updated_at_ms)
            .then_with(|| right.session_dir.cmp(&left.session_dir))
    });
    Ok(summaries)
}

pub fn list_workspace_session_summaries(
    config_root: &Path,
    workspace_path: &Path,
) -> Result<Vec<AppSessionSummary>> {
    let workspace_dir = config_root
        .join("sessions")
        .join(encode_workspace_path(workspace_path)?);
    let session_dirs = match std::fs::read_dir(&workspace_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", workspace_dir.display()));
        }
    };

    let mut summaries = Vec::new();
    for session_entry in session_dirs {
        let session_entry = session_entry?;
        if !session_entry.file_type()?.is_dir() {
            continue;
        }
        let summary = session_summary_from_dir(session_entry.path())?;
        if summary.message_count > 0 {
            summaries.push(summary);
        }
    }
    summaries.sort_by(|left, right| {
        right
            .updated_at_ms
            .cmp(&left.updated_at_ms)
            .then_with(|| right.session_dir.cmp(&left.session_dir))
    });
    Ok(summaries)
}

pub async fn delete_workspace_session(
    config_root: &Path,
    workspace_path: &Path,
    session_path: PathBuf,
) -> Result<bool> {
    let session_dir = normalize_session_dir_path(session_path)?;
    let metadata = match std::fs::symlink_metadata(&session_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect {}", session_dir.display()));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!(
            "session path is not a directory: {}",
            session_dir.display()
        ));
    }

    let workspace_dir = config_root
        .join("sessions")
        .join(encode_workspace_path(workspace_path)?);
    let workspace_root = std::fs::canonicalize(&workspace_dir)
        .with_context(|| format!("failed to resolve {}", workspace_dir.display()))?;
    let target = std::fs::canonicalize(&session_dir)
        .with_context(|| format!("failed to resolve {}", session_dir.display()))?;
    if target.parent() != Some(workspace_root.as_path()) {
        return Err(anyhow!(
            "session path is outside current workspace sessions: {}",
            session_dir.display()
        ));
    }
    resolve_session_identity(&target)?;

    tokio::fs::remove_dir_all(&target)
        .await
        .with_context(|| format!("failed to delete {}", target.display()))?;
    Ok(true)
}

pub fn normalize_session_dir_path(session_path: PathBuf) -> Result<PathBuf> {
    if session_path.file_name().and_then(|name| name.to_str())
        == Some(crate::core::session_journal::JOURNAL_FILE)
    {
        return session_path
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("journal.jsonl path has no parent session dir"));
    }
    Ok(session_path)
}

pub fn canonicalize_session_dir_path(session_path: PathBuf) -> Result<PathBuf> {
    let session_dir = normalize_session_dir_path(session_path)?;
    if let Ok(canonical) = std::fs::canonicalize(&session_dir) {
        return Ok(canonical);
    }
    if let (Some(parent), Some(name)) = (session_dir.parent(), session_dir.file_name())
        && let Ok(canonical_parent) = std::fs::canonicalize(parent)
    {
        return Ok(canonical_parent.join(name));
    }
    Ok(session_dir)
}

fn session_summary_from_dir(session_dir: PathBuf) -> Result<AppSessionSummary> {
    let session_id = resolve_session_identity(&session_dir)?.session_id;
    let workspace_path = workspace_path_from_session_dir(&session_dir)?;
    let projection = JournalProjection::build(session_id, load_records(&session_dir, session_id)?)?;
    let (message_count, preview) = messages_summary(&projection.history);
    let updated_at_ms = session_updated_at_ms(&journal_path(&session_dir));

    Ok(AppSessionSummary::new(
        session_dir,
        session_id,
        workspace_path,
        message_count,
        updated_at_ms,
        preview,
    ))
}

fn messages_summary(messages: &[CanonicalMessage]) -> (usize, Option<String>) {
    let mut first_text_preview = None;
    let mut first_user_preview = None;
    for message in messages {
        if let Some(text) = message_text_preview(message) {
            if first_text_preview.is_none() {
                first_text_preview = Some(text.clone());
            }
            if message.role == MessageRole::User && first_user_preview.is_none() {
                first_user_preview = Some(text);
            }
        }
    }
    (messages.len(), first_user_preview.or(first_text_preview))
}

fn message_text_preview(message: &CanonicalMessage) -> Option<String> {
    message.parts.iter().find_map(|part| match &part.payload {
        ContentPart::Text { text }
        | ContentPart::ReasoningSummary { text }
        | ContentPart::Reasoning { text, signature: _ } => {
            let text = text.trim();
            (!text.is_empty()).then(|| truncate_preview(text))
        }
        ContentPart::ToolResult { result } => {
            let text = result.text_or_status();
            let text = text.trim();
            (!text.is_empty()).then(|| truncate_preview(text))
        }
        _ => None,
    })
}

fn truncate_preview(text: &str) -> String {
    let limit = 160;
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        format!("{}...", text.chars().take(limit).collect::<String>())
    }
}

fn session_updated_at_ms(path: &Path) -> Option<u64> {
    path.metadata()
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
}

fn writer_for_session_dir(session_dir: &Path) -> Arc<Mutex<JournalWriterState>> {
    static WRITERS: OnceLock<StdMutex<HashMap<PathBuf, Arc<Mutex<JournalWriterState>>>>> =
        OnceLock::new();
    let key = canonicalize_journal_path(&journal_path(session_dir));
    let writers = WRITERS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut writers = writers.lock().expect("session journal writer map poisoned");
    writers
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(JournalWriterState::default())))
        .clone()
}

fn canonicalize_journal_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(canonical_parent) = std::fs::canonicalize(parent)
    {
        return canonical_parent.join(name);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests;
