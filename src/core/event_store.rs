use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};

use crate::{
    contracts::EventSink,
    domain::{Event, EventRecord, new_event_id},
};

#[derive(Debug)]
pub struct JsonlEventStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl JsonlEventStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }
}

#[async_trait::async_trait]
impl EventSink for JsonlEventStore {
    async fn append(&self, event: Event) -> Result<()> {
        let _guard = self.lock.lock().await;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let record = EventRecord {
            id: new_event_id(),
            event,
        };
        let mut line = serde_json::to_vec(&record)?;
        line.push(b'\n');

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .with_context(|| format!("failed to open event log {}", self.path.display()))?;
        file.write_all(&line).await?;
        file.flush().await?;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct InMemoryEventStore {
    events: Mutex<Vec<Event>>,
}

impl InMemoryEventStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn events(&self) -> Vec<Event> {
        self.events.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl EventSink for InMemoryEventStore {
    async fn append(&self, event: Event) -> Result<()> {
        self.events.lock().await.push(event);
        Ok(())
    }
}

impl From<InMemoryEventStore> for Arc<dyn EventSink> {
    fn from(store: InMemoryEventStore) -> Self {
        Arc::new(store)
    }
}
