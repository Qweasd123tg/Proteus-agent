use std::path::PathBuf;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::{
    fs::OpenOptions,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Mutex,
};

use crate::{
    contracts::MemoryStore,
    domain::{MemoryItem, MemoryQuery},
};

#[derive(Debug)]
pub struct JsonlMemory {
    path: PathBuf,
    lock: Mutex<()>,
}

impl JsonlMemory {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }
}

#[async_trait]
impl MemoryStore for JsonlMemory {
    async fn remember(&self, item: MemoryItem) -> Result<()> {
        let _guard = self.lock.lock().await;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .with_context(|| format!("failed to open memory {}", self.path.display()))?;
        let mut line = serde_json::to_vec(&item)?;
        line.push(b'\n');
        file.write_all(&line).await?;
        file.flush().await?;
        Ok(())
    }

    async fn recall(&self, query: MemoryQuery) -> Result<Vec<MemoryItem>> {
        let file = match OpenOptions::new().read(true).open(&self.path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut lines = BufReader::new(file).lines();
        let mut items = Vec::new();
        while let Some(line) = lines.next_line().await? {
            let item: MemoryItem = match serde_json::from_str(&line) {
                Ok(item) => item,
                Err(_) => continue,
            };
            if query.text.is_empty() || item.content.contains(&query.text) {
                items.push(item);
            }
            if items.len() >= query.limit {
                break;
            }
        }
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        contracts::MemoryStore,
        domain::{MemoryItem, MemoryQuery},
    };

    use super::*;

    #[tokio::test]
    async fn recall_skips_malformed_jsonl_lines() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("memory.jsonl");
        let first = MemoryItem::new("decision", "keep this", serde_json::Value::Null);
        let second = MemoryItem::new("preference", "keep that", serde_json::Value::Null);
        let contents = format!(
            "{}\nnot-json\n{}\n",
            serde_json::to_string(&first).expect("first item"),
            serde_json::to_string(&second).expect("second item")
        );
        tokio::fs::write(&path, contents)
            .await
            .expect("memory file");

        let memory = JsonlMemory::new(path);
        let items = memory
            .recall(MemoryQuery::new("keep", 10))
            .await
            .expect("recall");

        assert_eq!(items, vec![first, second]);
    }
}
