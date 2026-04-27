use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::domain::{Event, EventContext, EventEnvelope};

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn append(&self, envelope: EventEnvelope) -> Result<()>;
}

pub struct EventEmitter {
    sink: Arc<dyn EventSink>,
    seq: Mutex<u64>,
}

impl EventEmitter {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self {
            sink,
            seq: Mutex::new(0),
        }
    }

    pub async fn emit(&self, context: EventContext, event: Event) -> Result<()> {
        let mut seq = self.seq.lock().await;
        *seq += 1;
        let envelope = EventEnvelope::new(context, *seq, event);
        self.sink.append(envelope).await
    }
}
