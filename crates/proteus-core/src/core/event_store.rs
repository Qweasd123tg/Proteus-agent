use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, OnceLock},
};

use anyhow::{Context, Result};
use tokio::{
    fs::OpenOptions,
    io::AsyncWriteExt,
    sync::{Mutex, broadcast},
};

use crate::{
    contracts::EventSink,
    domain::{Event, EventEnvelope},
};

#[derive(Debug)]
pub struct JsonlEventStore {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl JsonlEventStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            lock: lock_for_path(&path),
            path,
        }
    }
}

#[async_trait::async_trait]
impl EventSink for JsonlEventStore {
    async fn append(&self, envelope: EventEnvelope) -> Result<()> {
        let _guard = self.lock.lock().await;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let mut line = serde_json::to_vec(&envelope)?;
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

fn lock_for_path(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<StdMutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let key = canonicalize_lock_path(path);
    let locks = LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut locks = locks.lock().expect("event store lock map poisoned");
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn canonicalize_lock_path(path: &Path) -> PathBuf {
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

#[derive(Debug, Default)]
pub struct InMemoryEventStore {
    events: Mutex<Vec<EventEnvelope>>,
}

impl InMemoryEventStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn events(&self) -> Vec<Event> {
        self.events
            .lock()
            .await
            .iter()
            .map(|envelope| envelope.event.clone())
            .collect()
    }

    pub async fn envelopes(&self) -> Vec<EventEnvelope> {
        self.events.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl EventSink for InMemoryEventStore {
    async fn append(&self, envelope: EventEnvelope) -> Result<()> {
        self.events.lock().await.push(envelope);
        Ok(())
    }
}

impl From<InMemoryEventStore> for Arc<dyn EventSink> {
    fn from(store: InMemoryEventStore) -> Self {
        Arc::new(store)
    }
}

/// Broadcasts every event to any number of subscribers. Lagging receivers
/// miss old events (tokio broadcast semantics) but the sink itself never
/// blocks or errors because of a slow consumer — `append` always returns Ok.
#[derive(Debug)]
pub struct BroadcastEventSink {
    tx: broadcast::Sender<EventEnvelope>,
}

impl BroadcastEventSink {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.tx.subscribe()
    }
}

#[async_trait::async_trait]
impl EventSink for BroadcastEventSink {
    async fn append(&self, envelope: EventEnvelope) -> Result<()> {
        let _ = self.tx.send(envelope);
        Ok(())
    }
}

/// Fan-out sink: forwards every event to an ordered list of inner sinks.
/// If any inner sink fails, the first error is returned — but all inner
/// sinks still receive the event first (best-effort delivery).
#[derive(Clone)]
pub struct FanoutEventSink {
    sinks: Vec<Arc<dyn EventSink>>,
}

impl FanoutEventSink {
    pub fn new(sinks: Vec<Arc<dyn EventSink>>) -> Self {
        Self { sinks }
    }
}

#[async_trait::async_trait]
impl EventSink for FanoutEventSink {
    async fn append(&self, envelope: EventEnvelope) -> Result<()> {
        let mut first_err: Option<anyhow::Error> = None;
        for sink in &self.sinks {
            if let Err(err) = sink.append(envelope.clone()).await
                && first_err.is_none()
            {
                first_err = Some(err);
            }
        }
        match first_err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}
