use anyhow::Result;
use async_trait::async_trait;

use crate::domain::Event;

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn append(&self, event: Event) -> Result<()>;
}
