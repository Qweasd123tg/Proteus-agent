use anyhow::Result;
use async_trait::async_trait;

use crate::domain::{MemoryItem, MemoryQuery};

#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn remember(&self, item: MemoryItem) -> Result<()>;
    async fn recall(&self, query: MemoryQuery) -> Result<Vec<MemoryItem>>;
}
