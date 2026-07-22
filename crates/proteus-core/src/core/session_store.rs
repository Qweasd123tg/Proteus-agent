use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result, anyhow, bail};
use proteus_contracts::app_protocol::AppSessionSummary;
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};
use uuid::Uuid;

use crate::{
    domain::SessionId,
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

mod workspace_dir;

use workspace_dir::workspace_path_from_session_dir;
pub use workspace_dir::{decode_workspace_path, encode_workspace_path};

const MESSAGES_FILE: &str = "messages.jsonl";
const PRE_COMPACTION_PREFIX: &str = "messages.pre-compaction.";
const PRE_COMPACTION_SUFFIX: &str = ".jsonl";

#[derive(Debug, Clone)]
pub struct SessionStore {
    session_dir: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl SessionStore {
    pub fn new(config_dir: &Path, cwd: &Path, session_id: SessionId) -> Result<Self> {
        let workspace = encode_workspace_path(cwd)?;
        let session_dir = config_dir
            .join("sessions")
            .join(workspace)
            .join(session_id.to_string());
        let lock = lock_for_session_dir(&session_dir);
        Ok(Self { session_dir, lock })
    }

    pub fn open(session_dir: PathBuf) -> Result<Self> {
        require_session_dir(&session_dir)?;
        workspace_path_from_session_dir(&session_dir)?;
        let lock = lock_for_session_dir(&session_dir);
        Ok(Self { session_dir, lock })
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    pub fn session_id(&self) -> SessionId {
        session_id_from_dir(&self.session_dir)
            .expect("SessionStore session directory was validated at construction")
    }

    pub fn workspace_path(&self) -> Result<PathBuf> {
        workspace_path_from_session_dir(&self.session_dir)
    }

    fn messages_path(&self) -> PathBuf {
        self.session_dir.join(MESSAGES_FILE)
    }

    pub fn load_messages(&self) -> Result<Vec<CanonicalMessage>> {
        let messages_path = self.messages_path();
        let content = match std::fs::read_to_string(&messages_path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", messages_path.display()));
            }
        };

        content
            .lines()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(index, line)| {
                serde_json::from_str::<CanonicalMessage>(line).with_context(|| {
                    format!(
                        "failed to parse {} line {}",
                        messages_path.display(),
                        index + 1
                    )
                })
            })
            .collect()
    }

    pub async fn append_messages(&self, messages: &[CanonicalMessage]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let _guard = self.lock.lock().await;
        tokio::fs::create_dir_all(&self.session_dir)
            .await
            .with_context(|| {
                format!(
                    "failed to create session dir {}",
                    self.session_dir.display()
                )
            })?;
        let messages_path = self.messages_path();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&messages_path)
            .await
            .with_context(|| format!("failed to open {}", messages_path.display()))?;

        for message in messages {
            let mut line = serde_json::to_vec(message)?;
            line.push(b'\n');
            file.write_all(&line).await?;
        }
        file.flush().await?;
        Ok(())
    }

    pub async fn replace_messages(&self, messages: &[CanonicalMessage]) -> Result<()> {
        let _guard = self.lock.lock().await;
        tokio::fs::create_dir_all(&self.session_dir)
            .await
            .with_context(|| {
                format!(
                    "failed to create session dir {}",
                    self.session_dir.display()
                )
            })?;
        let messages_path = self.messages_path();
        let tmp_path = messages_path.with_extension(format!("jsonl.tmp.{}", Uuid::new_v4()));
        let mut content = Vec::new();
        for message in messages {
            let mut line = serde_json::to_vec(message)?;
            line.push(b'\n');
            content.extend(line);
        }
        tokio::fs::write(&tmp_path, content)
            .await
            .with_context(|| format!("failed to write {}", tmp_path.display()))?;

        self.archive_messages_before_compaction().await?;

        tokio::fs::rename(&tmp_path, &messages_path)
            .await
            .with_context(|| {
                format!(
                    "failed to replace {} with {}",
                    messages_path.display(),
                    tmp_path.display()
                )
            })?;
        Ok(())
    }

    async fn archive_messages_before_compaction(&self) -> Result<()> {
        let messages_path = self.messages_path();
        match tokio::fs::metadata(&messages_path).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect {}", messages_path.display()));
            }
        }

        let seq = next_pre_compaction_archive_seq(&self.session_dir).await?;
        let archive_path = self.session_dir.join(format!(
            "{PRE_COMPACTION_PREFIX}{seq}{PRE_COMPACTION_SUFFIX}"
        ));
        tokio::fs::rename(&messages_path, &archive_path)
            .await
            .with_context(|| {
                format!(
                    "failed to archive {} as {} before compaction",
                    messages_path.display(),
                    archive_path.display()
                )
            })?;
        Ok(())
    }

    pub async fn clear(&self) -> Result<()> {
        let _guard = self.lock.lock().await;
        let messages_path = self.messages_path();
        if tokio::fs::try_exists(&messages_path).await? {
            tokio::fs::write(&messages_path, b"").await?;
        }
        Ok(())
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
    require_session_dir(&target)?;

    tokio::fs::remove_dir_all(&target)
        .await
        .with_context(|| format!("failed to delete {}", target.display()))?;
    Ok(true)
}

pub fn normalize_session_dir_path(session_path: PathBuf) -> Result<PathBuf> {
    if session_path.file_name().and_then(|name| name.to_str()) == Some(MESSAGES_FILE) {
        return session_path
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("messages.jsonl path has no parent session dir"));
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
    let session_id = require_session_dir(&session_dir)?;
    let workspace_path = workspace_path_from_session_dir(&session_dir)?;
    let (message_count, preview) = messages_summary(&session_dir.join(MESSAGES_FILE))?;
    let updated_at_ms = session_updated_at_ms(&session_dir.join(MESSAGES_FILE));

    Ok(AppSessionSummary::new(
        session_dir,
        session_id,
        workspace_path,
        message_count,
        updated_at_ms,
        preview,
    ))
}

fn messages_summary(messages_path: &Path) -> Result<(usize, Option<String>)> {
    let content = match std::fs::read_to_string(messages_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((0, None)),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", messages_path.display()));
        }
    };

    let mut count = 0;
    let mut first_text_preview = None;
    let mut first_user_preview = None;
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        count += 1;
        if let Ok(message) = serde_json::from_str::<CanonicalMessage>(line)
            && let Some(text) = message_text_preview(&message)
        {
            if first_text_preview.is_none() {
                first_text_preview = Some(text.clone());
            }
            if message.role == MessageRole::User && first_user_preview.is_none() {
                first_user_preview = Some(text);
            }
        }
    }
    Ok((count, first_user_preview.or(first_text_preview)))
}

fn message_text_preview(message: &CanonicalMessage) -> Option<String> {
    message.parts.iter().find_map(|part| match part {
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

async fn next_pre_compaction_archive_seq(session_dir: &Path) -> Result<u64> {
    let mut max_seq = 0_u64;
    let mut entries = tokio::fs::read_dir(session_dir)
        .await
        .with_context(|| format!("failed to read {}", session_dir.display()))?;
    while let Some(entry) = entries.next_entry().await? {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(seq) = pre_compaction_archive_seq(&name) else {
            continue;
        };
        max_seq = max_seq.max(seq);
    }
    Ok(max_seq.saturating_add(1))
}

fn pre_compaction_archive_seq(file_name: &str) -> Option<u64> {
    file_name
        .strip_prefix(PRE_COMPACTION_PREFIX)?
        .strip_suffix(PRE_COMPACTION_SUFFIX)?
        .parse::<u64>()
        .ok()
}

fn require_session_dir(session_dir: &Path) -> Result<SessionId> {
    let metadata = std::fs::metadata(session_dir).with_context(|| {
        format!(
            "failed to inspect session directory {}",
            session_dir.display()
        )
    })?;
    if !metadata.is_dir() {
        bail!("session path is not a directory: {}", session_dir.display());
    }
    session_id_from_dir(session_dir)
}

fn session_id_from_dir(session_dir: &Path) -> Result<SessionId> {
    let actual_name = session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            anyhow!(
                "session directory must have a UTF-8 UUID basename: {}",
                session_dir.display()
            )
        })?;
    let session_id = actual_name.parse::<SessionId>().with_context(|| {
        format!(
            "incompatible session directory {}; basename must be a canonical UUID. Pre-release session formats are not migrated automatically; move this directory outside the sessions tree or remove it before retrying",
            session_dir.display()
        )
    })?;
    let canonical_name = session_id.to_string();
    if actual_name != canonical_name {
        bail!(
            "session directory basename {actual_name:?} is not canonical; expected {canonical_name:?}: {}",
            session_dir.display()
        );
    }
    Ok(session_id)
}

fn lock_for_session_dir(session_dir: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<StdMutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let key = canonicalize_message_lock_path(&session_dir.join(MESSAGES_FILE));
    let locks = LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut locks = locks.lock().expect("session store lock map poisoned");
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn canonicalize_message_lock_path(messages_path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(messages_path) {
        return canonical;
    }
    if let (Some(parent), Some(name)) = (messages_path.parent(), messages_path.file_name())
        && let Ok(canonical_parent) = std::fs::canonicalize(parent)
    {
        return canonical_parent.join(name);
    }
    messages_path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use crate::domain::new_session_id;
    use crate::model_standard::{CanonicalMessage, MessageRole};

    use super::*;

    #[test]
    fn open_reads_session_identity_from_directory() {
        let session_id = new_session_id();
        let config_dir = tempfile::tempdir().expect("config dir");
        let workspace = tempfile::tempdir().expect("workspace");
        let session_dir = test_session_dir(config_dir.path(), workspace.path(), session_id);
        std::fs::create_dir_all(&session_dir).expect("session dir");

        let store = SessionStore::open(session_dir).expect("open session");

        assert_eq!(store.session_id(), session_id);
        assert_eq!(store.workspace_path().expect("workspace"), workspace.path());
    }

    #[test]
    fn open_rejects_noncanonical_basename() {
        let session_id = new_session_id();
        let config_dir = tempfile::tempdir().expect("config dir");
        let workspace = tempfile::tempdir().expect("workspace");
        let session_dir = test_session_dir(config_dir.path(), workspace.path(), session_id)
            .with_file_name("1234567890");
        std::fs::create_dir_all(&session_dir).expect("session dir");

        let error = SessionStore::open(session_dir).expect_err("non-UUID basename must fail");

        assert!(error.to_string().contains("canonical UUID"));
    }

    #[test]
    fn open_requires_existing_session_directory() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let workspace = tempfile::tempdir().expect("workspace");
        let session_dir = test_session_dir(config_dir.path(), workspace.path(), new_session_id());
        let error = SessionStore::open(session_dir).expect_err("session dir must exist");

        assert!(
            error
                .to_string()
                .contains("failed to inspect session directory")
        );
    }

    #[test]
    fn normalize_session_dir_accepts_messages_jsonl_path() {
        let session_dir = PathBuf::from(new_session_id().to_string());
        let messages_path = session_dir.join("messages.jsonl");

        let normalized = normalize_session_dir_path(messages_path).expect("normalized");

        assert_eq!(normalized, session_dir);
    }

    #[test]
    fn session_dir_uses_full_session_id() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let session_id = new_session_id();

        let store = test_store(config_dir.path(), cwd.path(), session_id);
        let name = store
            .session_dir()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("session dir name");

        assert_eq!(name, session_id.to_string());
    }

    #[test]
    fn opened_session_without_messages_loads_empty_history() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let store = test_store(config_dir.path(), cwd.path(), new_session_id());
        std::fs::create_dir_all(store.session_dir()).expect("session dir");
        let store = SessionStore::open(store.session_dir().to_path_buf()).expect("open session");

        let messages = store.load_messages().expect("load messages");

        assert!(messages.is_empty());
    }

    #[tokio::test]
    async fn messages_round_trip_through_jsonl_store() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let store = test_store(config_dir.path(), cwd.path(), new_session_id());
        let messages = vec![
            CanonicalMessage::text(MessageRole::User, "hello"),
            CanonicalMessage::text(MessageRole::Assistant, "hi"),
        ];

        store
            .append_messages(&messages)
            .await
            .expect("append messages");
        let reopened = SessionStore::open(store.session_dir().to_path_buf()).expect("open session");
        let loaded = reopened.load_messages().expect("load messages");

        assert_eq!(loaded, messages);
    }

    #[tokio::test]
    async fn replace_messages_archives_pre_compaction_history_with_incrementing_seq() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let store = test_store(config_dir.path(), cwd.path(), new_session_id());
        let first = vec![CanonicalMessage::text(MessageRole::User, "before first")];
        let second = vec![CanonicalMessage::text(
            MessageRole::Assistant,
            "after first",
        )];
        let third = vec![CanonicalMessage::text(MessageRole::User, "after second")];

        store.append_messages(&first).await.expect("seed messages");
        store
            .replace_messages(&second)
            .await
            .expect("first replace");
        store
            .replace_messages(&third)
            .await
            .expect("second replace");

        let archive_one = store.session_dir().join("messages.pre-compaction.1.jsonl");
        let archive_two = store.session_dir().join("messages.pre-compaction.2.jsonl");
        assert!(archive_one.exists());
        assert!(archive_two.exists());
        assert_eq!(read_messages_file(&archive_one), first);
        assert_eq!(read_messages_file(&archive_two), second);
        assert_eq!(store.load_messages().expect("load current"), third);
    }

    #[tokio::test]
    async fn load_messages_ignores_pre_compaction_archives() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let store = test_store(config_dir.path(), cwd.path(), new_session_id());
        std::fs::create_dir_all(store.session_dir()).expect("session dir");
        let session_dir = store.session_dir();
        let archived = CanonicalMessage::text(MessageRole::User, "archived only");
        let mut line = serde_json::to_vec(&archived).expect("archive json");
        line.push(b'\n');
        std::fs::write(session_dir.join("messages.pre-compaction.1.jsonl"), line)
            .expect("write archive");

        let reopened = SessionStore::open(session_dir.to_path_buf()).expect("open session");
        let loaded = reopened.load_messages().expect("load messages");

        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn append_creates_only_messages_file() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let session_id = new_session_id();
        let store = test_store(config_dir.path(), cwd.path(), session_id);

        store
            .append_messages(&[CanonicalMessage::text(MessageRole::User, "hello")])
            .await
            .expect("append messages");
        let reopened = SessionStore::open(store.session_dir().to_path_buf()).expect("open session");
        let entries = std::fs::read_dir(store.session_dir())
            .expect("session directory")
            .map(|entry| {
                entry
                    .expect("session entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();

        assert_eq!(reopened.session_id(), session_id);
        assert_eq!(reopened.workspace_path().expect("workspace"), cwd.path());
        assert_eq!(entries, [MESSAGES_FILE.to_owned()]);
    }

    #[tokio::test]
    async fn renaming_workspace_directory_changes_session_workspace() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let original_workspace = tempfile::tempdir().expect("original workspace");
        let moved_workspace = tempfile::tempdir().expect("moved workspace");
        let session_id = new_session_id();
        let store = test_store(config_dir.path(), original_workspace.path(), session_id);
        store
            .append_messages(&[CanonicalMessage::text(MessageRole::User, "move me")])
            .await
            .expect("append messages");

        let original_workspace_dir = store
            .session_dir()
            .parent()
            .expect("workspace session dir")
            .to_path_buf();
        let moved_workspace_dir = config_dir
            .path()
            .join("sessions")
            .join(encode_workspace_path(moved_workspace.path()).expect("encoded workspace"));
        std::fs::rename(&original_workspace_dir, &moved_workspace_dir)
            .expect("rename workspace session dir");
        let moved_session_dir = moved_workspace_dir.join(session_id.to_string());

        let reopened = SessionStore::open(moved_session_dir).expect("open moved session");
        let old_summaries =
            list_workspace_session_summaries(config_dir.path(), original_workspace.path())
                .expect("old workspace sessions");
        let moved_summaries =
            list_workspace_session_summaries(config_dir.path(), moved_workspace.path())
                .expect("moved workspace sessions");

        assert_eq!(
            reopened.workspace_path().expect("moved workspace"),
            moved_workspace.path()
        );
        assert!(old_summaries.is_empty());
        assert_eq!(moved_summaries.len(), 1);
        assert_eq!(moved_summaries[0].workspace_path, moved_workspace.path());
    }

    #[test]
    fn empty_session_directory_is_not_listed() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let session_id = new_session_id();
        let store = test_store(config_dir.path(), cwd.path(), session_id);

        std::fs::create_dir_all(store.session_dir()).expect("session dir");

        assert!(!store.messages_path().exists());
        let reopened = SessionStore::open(store.session_dir().to_path_buf()).expect("open session");
        assert_eq!(reopened.session_id(), session_id);

        let summaries = list_workspace_session_summaries(config_dir.path(), cwd.path())
            .expect("workspace sessions");
        assert!(summaries.is_empty());
    }

    #[tokio::test]
    async fn delete_workspace_session_removes_only_current_workspace_session_dir() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let other_cwd = tempfile::tempdir().expect("other cwd");
        let session_id = new_session_id();
        let other_session_id = new_session_id();
        let store = test_store(config_dir.path(), cwd.path(), session_id);
        let other_store = test_store(config_dir.path(), other_cwd.path(), other_session_id);
        std::fs::create_dir_all(store.session_dir()).expect("session dir");
        std::fs::create_dir_all(other_store.session_dir()).expect("other session dir");

        let deleted = delete_workspace_session(
            config_dir.path(),
            cwd.path(),
            store.session_dir().to_path_buf(),
        )
        .await
        .expect("delete session");

        assert!(deleted);
        assert!(!store.session_dir().exists());
        assert!(other_store.session_dir().exists());
        let error = delete_workspace_session(
            config_dir.path(),
            cwd.path(),
            other_store.session_dir().to_path_buf(),
        )
        .await
        .expect_err("other workspace session must not be deleted");
        assert!(
            error
                .to_string()
                .contains("outside current workspace sessions")
        );
        assert!(other_store.session_dir().exists());
    }

    #[tokio::test]
    async fn list_session_summaries_returns_recent_sessions() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let session_id = new_session_id();
        let store = test_store(config_dir.path(), cwd.path(), session_id);
        store
            .append_messages(&[
                CanonicalMessage::text(MessageRole::User, "inspect this project"),
                CanonicalMessage::text(MessageRole::Assistant, "done"),
            ])
            .await
            .expect("append messages");

        let summaries = list_session_summaries(config_dir.path()).expect("sessions");

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].session_id, session_id);
        assert_eq!(summaries[0].workspace_path, cwd.path());
        assert_eq!(summaries[0].message_count, 2);
        assert_eq!(summaries[0].activity, None);
        assert_eq!(
            summaries[0].preview.as_deref(),
            Some("inspect this project")
        );
    }

    #[tokio::test]
    async fn list_workspace_session_summaries_filters_to_current_workspace() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let first_cwd = tempfile::tempdir().expect("first cwd");
        let second_cwd = tempfile::tempdir().expect("second cwd");
        let first_session_id = new_session_id();
        let second_session_id = new_session_id();
        let first_store = test_store(config_dir.path(), first_cwd.path(), first_session_id);
        let second_store = test_store(config_dir.path(), second_cwd.path(), second_session_id);
        first_store
            .append_messages(&[CanonicalMessage::text(MessageRole::User, "first workspace")])
            .await
            .expect("append first workspace");
        second_store
            .append_messages(&[CanonicalMessage::text(
                MessageRole::User,
                "second workspace",
            )])
            .await
            .expect("append second workspace");

        let summaries = list_workspace_session_summaries(config_dir.path(), first_cwd.path())
            .expect("workspace sessions");

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].session_id, first_session_id);
        assert_eq!(summaries[0].preview.as_deref(), Some("first workspace"));
    }

    #[test]
    fn list_session_summaries_rejects_non_uuid_session_directory() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let workspace = tempfile::tempdir().expect("workspace");
        let session_dir = config_dir
            .path()
            .join("sessions")
            .join(encode_workspace_path(workspace.path()).expect("encoded workspace"))
            .join("not-a-session-id");
        std::fs::create_dir_all(&session_dir).expect("session dir");
        std::fs::write(
            session_dir.join(MESSAGES_FILE),
            serde_json::to_string(&CanonicalMessage::text(MessageRole::User, "hello"))
                .expect("message json"),
        )
        .expect("messages file");

        let error =
            list_session_summaries(config_dir.path()).expect_err("invalid session id must fail");

        assert!(error.to_string().contains("canonical UUID"));
    }

    fn read_messages_file(path: &Path) -> Vec<CanonicalMessage> {
        std::fs::read_to_string(path)
            .expect("read messages file")
            .lines()
            .map(|line| serde_json::from_str(line).expect("message json"))
            .collect()
    }

    fn test_store(config_dir: &Path, workspace_path: &Path, session_id: SessionId) -> SessionStore {
        SessionStore::new(config_dir, workspace_path, session_id).expect("session store")
    }

    fn test_session_dir(
        config_dir: &Path,
        workspace_path: &Path,
        session_id: SessionId,
    ) -> PathBuf {
        config_dir
            .join("sessions")
            .join(encode_workspace_path(workspace_path).expect("encoded workspace"))
            .join(session_id.to_string())
    }
}
