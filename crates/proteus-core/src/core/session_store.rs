use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};

use crate::{
    domain::SessionId,
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

const SESSION_METADATA_FILE: &str = "session.json";

#[derive(Debug, Clone)]
pub struct SessionStore {
    session_dir: PathBuf,
    messages_path: PathBuf,
    metadata_path: PathBuf,
    session_id: Option<SessionId>,
    workspace_path: Option<PathBuf>,
    lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionMetadata {
    schema_version: u32,
    session_id: SessionId,
    #[serde(default)]
    workspace_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<PathBuf>,
    pub message_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    pub resumable: bool,
}

impl SessionStore {
    pub fn new(config_dir: &Path, cwd: &Path, session_id: SessionId) -> Self {
        let workspace = encode_workspace_path(cwd);
        let session_name = session_dir_name(session_id);
        let session_dir = config_dir
            .join("sessions")
            .join(workspace)
            .join(session_name);
        let messages_path = session_dir.join("messages.jsonl");
        let metadata_path = session_dir.join(SESSION_METADATA_FILE);
        Self {
            session_dir,
            messages_path,
            metadata_path,
            session_id: Some(session_id),
            workspace_path: Some(canonical_or_original(cwd)),
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn from_session_dir(session_dir: PathBuf) -> Self {
        let messages_path = session_dir.join("messages.jsonl");
        let metadata_path = session_dir.join(SESSION_METADATA_FILE);
        Self {
            session_dir,
            messages_path,
            metadata_path,
            session_id: None,
            workspace_path: None,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    pub fn load_messages(&self) -> Result<Vec<CanonicalMessage>> {
        let content = match std::fs::read_to_string(&self.messages_path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", self.messages_path.display()));
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
                        self.messages_path.display(),
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
        self.write_metadata_if_needed().await?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.messages_path)
            .await
            .with_context(|| format!("failed to open {}", self.messages_path.display()))?;

        for message in messages {
            let mut line = serde_json::to_vec(message)?;
            line.push(b'\n');
            file.write_all(&line).await?;
        }
        file.flush().await?;
        Ok(())
    }

    async fn write_metadata_if_needed(&self) -> Result<()> {
        let Some(session_id) = self.session_id else {
            return Ok(());
        };
        if tokio::fs::try_exists(&self.metadata_path).await? {
            return Ok(());
        }

        let metadata = SessionMetadata {
            schema_version: 2,
            session_id,
            workspace_path: self.workspace_path.clone(),
        };
        let mut content = serde_json::to_vec_pretty(&metadata)?;
        content.push(b'\n');
        tokio::fs::write(&self.metadata_path, content)
            .await
            .with_context(|| format!("failed to write {}", self.metadata_path.display()))?;
        Ok(())
    }

    pub async fn clear(&self) -> Result<()> {
        let _guard = self.lock.lock().await;
        if tokio::fs::try_exists(&self.messages_path).await? {
            tokio::fs::write(&self.messages_path, b"").await?;
        }
        Ok(())
    }
}

pub fn list_session_summaries(config_root: &Path) -> Result<Vec<SessionSummary>> {
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
            summaries.push(session_summary_from_dir(session_dir)?);
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

pub fn normalize_session_dir_path(session_path: PathBuf) -> Result<PathBuf> {
    if session_path.file_name().and_then(|name| name.to_str()) == Some("messages.jsonl") {
        return session_path
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("messages.jsonl path has no parent session dir"));
    }
    Ok(session_path)
}

pub fn session_id_from_session_dir(session_dir: &Path) -> Result<SessionId> {
    match read_session_metadata(session_dir)? {
        Some(metadata) => Ok(metadata.session_id),
        None => Err(anyhow!(
            "session metadata file is required: {}",
            session_dir.join(SESSION_METADATA_FILE).display()
        )),
    }
}

pub fn session_workspace_from_session_dir(session_dir: &Path) -> Result<Option<PathBuf>> {
    if let Some(metadata) = read_session_metadata(session_dir)?
        && let Some(workspace_path) = metadata.workspace_path
    {
        return Ok(Some(workspace_path));
    }

    Ok(infer_workspace_path_from_session_dir(session_dir))
}

fn session_summary_from_dir(session_dir: PathBuf) -> Result<SessionSummary> {
    let metadata = read_session_metadata(&session_dir)?;
    let workspace_path = metadata
        .as_ref()
        .and_then(|metadata| metadata.workspace_path.clone())
        .or_else(|| infer_workspace_path_from_session_dir(&session_dir));
    let (message_count, preview) = messages_summary(&session_dir.join("messages.jsonl"))?;
    let updated_at_ms = session_updated_at_ms(&session_dir.join("messages.jsonl"))
        .or_else(|| session_updated_at_ms(&session_dir.join(SESSION_METADATA_FILE)));

    Ok(SessionSummary {
        session_dir,
        session_id: metadata.as_ref().map(|metadata| metadata.session_id),
        workspace_path,
        message_count,
        updated_at_ms,
        preview,
        resumable: metadata.is_some(),
    })
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

fn read_session_metadata(session_dir: &Path) -> Result<Option<SessionMetadata>> {
    let metadata_path = session_dir.join(SESSION_METADATA_FILE);
    match std::fs::read_to_string(&metadata_path) {
        Ok(content) => serde_json::from_str(&content)
            .map(Some)
            .with_context(|| format!("failed to parse {}", metadata_path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read {}", metadata_path.display()))
        }
    }
}

fn infer_workspace_path_from_session_dir(session_dir: &Path) -> Option<PathBuf> {
    let encoded = session_dir.parent()?.file_name()?.to_str()?;
    let parts = encoded
        .split('|')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return None;
    }

    let mut path = PathBuf::from("/");
    for part in parts {
        path.push(part);
    }
    path.exists().then_some(path)
}

fn session_dir_name(session_id: SessionId) -> String {
    short_numeric_session_id(session_id)
}

fn short_numeric_session_id(session_id: SessionId) -> String {
    format!("{:010}", session_id.as_u128() % 10_000_000_000)
}

pub fn encode_workspace_path(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let parts = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(sanitize_path_part(&part.to_string_lossy())),
            _ => None,
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    if parts.is_empty() {
        "root".to_owned()
    } else {
        parts.join("|")
    }
}

fn sanitize_path_part(input: &str) -> String {
    let mut out = String::new();
    for ch in input.trim().chars() {
        if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }

    while out.contains("__") {
        out = out.replace("__", "_");
    }

    out.trim_matches('_').to_owned()
}

fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use crate::domain::new_session_id;
    use crate::model_standard::{CanonicalMessage, MessageRole};

    use super::*;

    #[test]
    fn session_id_from_dir_reads_metadata() {
        let session_id = new_session_id();
        let dir = tempfile::tempdir().expect("session dir");
        std::fs::write(
            dir.path().join(SESSION_METADATA_FILE),
            serde_json::to_string(&SessionMetadata {
                schema_version: 1,
                session_id,
                workspace_path: None,
            })
            .expect("metadata json"),
        )
        .expect("metadata file");

        let parsed = session_id_from_session_dir(dir.path()).expect("session id");

        assert_eq!(parsed, session_id);
    }

    #[test]
    fn session_id_from_dir_requires_metadata() {
        let error = session_id_from_session_dir(Path::new("1234567890"))
            .expect_err("session dir needs metadata");

        assert!(
            error
                .to_string()
                .contains("session metadata file is required")
        );
    }

    #[test]
    fn normalize_session_dir_accepts_messages_jsonl_path() {
        let session_dir = PathBuf::from("1234567890");
        let messages_path = session_dir.join("messages.jsonl");

        let normalized = normalize_session_dir_path(messages_path).expect("normalized");

        assert_eq!(normalized, session_dir);
    }

    #[test]
    fn session_dir_is_short_numeric_id_only() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let cwd_label = cwd.path().file_name().unwrap().to_string_lossy();
        let session_id = new_session_id();

        let store = SessionStore::new(config_dir.path(), cwd.path(), session_id);
        let name = store
            .session_dir()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("session dir name");

        assert!(!name.contains(cwd_label.as_ref()));
        assert_eq!(name.len(), 10);
        assert!(name.chars().all(|ch| ch.is_ascii_digit()));
    }

    #[test]
    fn missing_messages_file_loads_empty_history() {
        let dir = tempfile::tempdir().expect("session dir");
        let store = SessionStore::from_session_dir(dir.path().join("missing-session"));

        let messages = store.load_messages().expect("load messages");

        assert!(messages.is_empty());
    }

    #[tokio::test]
    async fn messages_round_trip_through_jsonl_store() {
        let dir = tempfile::tempdir().expect("session dir");
        let store = SessionStore::from_session_dir(dir.path().join("session"));
        let messages = vec![
            CanonicalMessage::text(MessageRole::User, "hello"),
            CanonicalMessage::text(MessageRole::Assistant, "hi"),
        ];

        store
            .append_messages(&messages)
            .await
            .expect("append messages");
        let loaded = store.load_messages().expect("load messages");

        assert_eq!(loaded, messages);
    }

    #[tokio::test]
    async fn append_writes_session_metadata_for_new_store() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let session_id = new_session_id();
        let store = SessionStore::new(config_dir.path(), cwd.path(), session_id);

        store
            .append_messages(&[CanonicalMessage::text(MessageRole::User, "hello")])
            .await
            .expect("append messages");
        let parsed = session_id_from_session_dir(store.session_dir()).expect("metadata id");
        let workspace =
            session_workspace_from_session_dir(store.session_dir()).expect("metadata workspace");

        assert_eq!(parsed, session_id);
        assert_eq!(workspace.as_deref(), Some(cwd.path()));
    }

    #[tokio::test]
    async fn list_session_summaries_returns_recent_resumable_sessions() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let session_id = new_session_id();
        let store = SessionStore::new(config_dir.path(), cwd.path(), session_id);
        store
            .append_messages(&[
                CanonicalMessage::text(MessageRole::User, "inspect this project"),
                CanonicalMessage::text(MessageRole::Assistant, "done"),
            ])
            .await
            .expect("append messages");

        let summaries = list_session_summaries(config_dir.path()).expect("sessions");

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].session_id, Some(session_id));
        assert_eq!(summaries[0].workspace_path.as_deref(), Some(cwd.path()));
        assert_eq!(summaries[0].message_count, 2);
        assert_eq!(
            summaries[0].preview.as_deref(),
            Some("inspect this project")
        );
        assert!(summaries[0].resumable);
    }

    #[test]
    fn session_workspace_infers_existing_path_from_legacy_session_layout() {
        let dir = tempfile::tempdir().expect("root");
        let workspace = dir.path().join("тест").join("ветер");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let encoded = encode_workspace_path(&workspace);
        let session_dir = dir
            .path()
            .join("config")
            .join("sessions")
            .join(encoded)
            .join("1234567890");

        let inferred = infer_workspace_path_from_session_dir(&session_dir);

        assert_eq!(inferred.as_deref(), Some(workspace.as_path()));
    }
}
