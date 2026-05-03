use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use chrono::Local;
use serde::{Deserialize, Serialize};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};

use crate::{domain::SessionId, model_standard::CanonicalMessage};

const SESSION_METADATA_FILE: &str = "session.json";

#[derive(Debug, Clone)]
pub struct SessionStore {
    session_dir: PathBuf,
    messages_path: PathBuf,
    metadata_path: PathBuf,
    session_id: Option<SessionId>,
    lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionMetadata {
    schema_version: u32,
    session_id: SessionId,
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
            schema_version: 1,
            session_id,
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
    let metadata_path = session_dir.join(SESSION_METADATA_FILE);
    match std::fs::read_to_string(&metadata_path) {
        Ok(content) => {
            let metadata: SessionMetadata = serde_json::from_str(&content)
                .with_context(|| format!("failed to parse {}", metadata_path.display()))?;
            return Ok(metadata.session_id);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", metadata_path.display()));
        }
    }

    let name = session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            anyhow!(
                "session dir has no UTF-8 file name: {}",
                session_dir.display()
            )
        })?;
    let raw_id = name
        .rsplit('|')
        .next()
        .filter(|part| !part.is_empty())
        .ok_or_else(|| anyhow!("session dir name does not contain session id: {name}"))?;
    uuid::Uuid::parse_str(raw_id)
        .with_context(|| format!("failed to parse session id from session dir name: {name}"))
}

fn session_dir_name(session_id: SessionId) -> String {
    format!(
        "{}-{}",
        Local::now().format("%Y%m%d-%H%M%S"),
        short_numeric_session_id(session_id)
    )
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
            })
            .expect("metadata json"),
        )
        .expect("metadata file");

        let parsed = session_id_from_session_dir(dir.path()).expect("session id");

        assert_eq!(parsed, session_id);
    }

    #[test]
    fn session_id_from_dir_reads_legacy_uuid_suffix() {
        let session_id = new_session_id();
        let session_dir = PathBuf::from(format!("workspace|20260503-120000|{session_id}"));

        let parsed = session_id_from_session_dir(&session_dir).expect("session id");

        assert_eq!(parsed, session_id);
    }

    #[test]
    fn session_id_from_dir_rejects_new_short_name_without_metadata() {
        let error = session_id_from_session_dir(Path::new("20260503-120000-1234567890"))
            .expect_err("short session dir needs metadata");

        assert!(
            error
                .to_string()
                .contains("failed to parse session id from session dir name")
        );
    }

    #[test]
    fn normalize_session_dir_accepts_messages_jsonl_path() {
        let session_dir = PathBuf::from("workspace|20260503-120000|session-id");
        let messages_path = session_dir.join("messages.jsonl");

        let normalized = normalize_session_dir_path(messages_path).expect("normalized");

        assert_eq!(normalized, session_dir);
    }

    #[test]
    fn session_dir_omits_workspace_label_and_uses_short_numeric_suffix() {
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
        let short_id = name.rsplit('-').next().expect("numeric id");

        assert!(!name.contains(cwd_label.as_ref()));
        assert_eq!(short_id.len(), 10);
        assert!(short_id.chars().all(|ch| ch.is_ascii_digit()));
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

        assert_eq!(parsed, session_id);
    }
}
