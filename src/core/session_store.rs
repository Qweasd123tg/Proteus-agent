use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use chrono::Local;
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};

use crate::model_standard::CanonicalMessage;

#[derive(Debug, Clone)]
pub struct SessionStore {
    session_dir: PathBuf,
    messages_path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl SessionStore {
    pub fn new(config_dir: &Path, cwd: &Path) -> Self {
        let workspace = encode_workspace_path(cwd);
        let session_name = format!(
            "{}|{}",
            session_label(cwd),
            Local::now().format("%Y%m%d-%H%M%S")
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

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
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
