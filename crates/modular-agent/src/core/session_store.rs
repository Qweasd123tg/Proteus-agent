use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use chrono::Local;
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};

use crate::{domain::SessionId, model_standard::CanonicalMessage};

#[derive(Debug, Clone)]
pub struct SessionStore {
    session_dir: PathBuf,
    messages_path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl SessionStore {
    pub fn new(config_dir: &Path, cwd: &Path, session_id: SessionId) -> Self {
        let workspace = encode_workspace_path(cwd);
        let session_name = format!(
            "{}|{}|{}",
            session_label(cwd),
            Local::now().format("%Y%m%d-%H%M%S"),
            session_id
        );
        let session_dir = config_dir
            .join("sessions")
            .join(workspace)
            .join(session_name);
        let messages_path = session_dir.join("messages.jsonl");
        Self {
            session_dir,
            messages_path,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn from_session_dir(session_dir: PathBuf) -> Self {
        let messages_path = session_dir.join("messages.jsonl");
        Self {
            session_dir,
            messages_path,
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

fn session_label(cwd: &Path) -> String {
    cwd.file_name()
        .map(|name| sanitize_path_part(&name.to_string_lossy()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "session".to_owned())
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
    fn session_id_from_dir_reads_suffix_after_pipe() {
        let session_id = new_session_id();
        let session_dir = PathBuf::from(format!("workspace|20260503-120000|{session_id}"));

        let parsed = session_id_from_session_dir(&session_dir).expect("session id");

        assert_eq!(parsed, session_id);
    }

    #[test]
    fn session_id_from_dir_rejects_missing_uuid_suffix() {
        let error =
            session_id_from_session_dir(Path::new("workspace|20260503-120000|")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("session dir name does not contain session id")
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
    fn session_dir_includes_session_id_to_avoid_same_second_collisions() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");

        let first = SessionStore::new(config_dir.path(), cwd.path(), new_session_id());
        let second = SessionStore::new(config_dir.path(), cwd.path(), new_session_id());

        assert_ne!(first.session_dir(), second.session_dir());
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
}
