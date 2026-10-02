use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, OnceLock},
};

use anyhow::{Context, Result, anyhow};
use tokio::sync::Mutex;

use crate::{
    contracts::ExecutionAttribution,
    core::session_journal::{
        DEFAULT_BLOB_THRESHOLD_BYTES, HistoryMutated, HistoryMutationKind, JournalEntry,
        JournalProjection, JournalRecord, JournalRecordAttribution, JournalWriterState,
        append_record, initialize_writer_state, journal_path, load_records,
    },
    domain::{ExecutionId, HistoryCompactionReport, SessionId, ThreadId, TurnId},
    model_standard::CanonicalMessage,
};

#[cfg(test)]
use crate::model_standard::{ContentPart, MessageRole};

mod catalog;
mod checkpoint;
mod identity;
mod usage;
mod workspace_dir;

pub use catalog::{
    list_session_summaries, list_session_summaries_for_audit, list_workspace_session_summaries,
};
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

    /// Admission may write image bytes before the first journal record. Create
    /// the canonical session identity and acquire its writer lease first.
    pub(crate) async fn prepare_attachments(&self) -> Result<()> {
        let mut writer = self.writer.lock().await;
        self.materialize_for_write().await?;
        initialize_writer_state(&self.session_dir, self.session_id, &mut writer)
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
                tool_results: Vec::new(),
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
                tool_results: Vec::new(),
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
