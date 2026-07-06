use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};

use crate::{domain::ThreadId, model_standard::CanonicalModelRequest};

pub const REQUEST_SNAPSHOTS_FILE: &str = "requests.jsonl";

#[derive(Debug)]
pub struct RequestSnapshotWriter {
    path: PathBuf,
    lock: Mutex<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSnapshotRecord {
    pub schema_version: u32,
    pub ts: u64,
    pub thread_id: ThreadId,
    pub request: CanonicalModelRequest,
}

impl RequestSnapshotWriter {
    pub fn new(session_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: session_dir.into().join(REQUEST_SNAPSHOTS_FILE),
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn append(&self, thread_id: ThreadId, request: &CanonicalModelRequest) -> Result<()> {
        let _guard = self.lock.lock().await;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create session dir {}", parent.display()))?;
        }
        let record = RequestSnapshotRecord {
            schema_version: 1,
            ts: unix_timestamp_ms(),
            thread_id,
            request: request.clone(),
        };
        let mut line = serde_json::to_vec(&record)?;
        line.push(b'\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .with_context(|| format!("failed to open {}", self.path.display()))?;
        file.write_all(&line).await?;
        file.flush().await?;
        Ok(())
    }
}

pub fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::ModelRef,
        model_standard::{CanonicalMessage, MessageRole},
    };

    #[tokio::test]
    async fn writer_appends_valid_jsonl_request_snapshot() {
        let dir = tempfile::tempdir().expect("session dir");
        let thread_id = crate::domain::new_thread_id();
        let request = CanonicalModelRequest::new(
            ModelRef::new("fake", "snapshot-model"),
            vec![CanonicalMessage::text(MessageRole::User, "hello")],
        );
        let writer = RequestSnapshotWriter::new(dir.path());

        writer
            .append(thread_id, &request)
            .await
            .expect("append request snapshot");

        let content = std::fs::read_to_string(writer.path()).expect("read requests jsonl");
        let lines = content.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 1);
        let record: RequestSnapshotRecord =
            serde_json::from_str(lines[0]).expect("request snapshot json");
        assert_eq!(record.schema_version, 1);
        assert_eq!(record.thread_id, thread_id);
        assert_eq!(record.request, request);
        assert!(record.ts > 0);
    }
}
