use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::MemoryStore,
    domain::{MemoryItem, MemoryQuery},
};

#[derive(Debug)]
pub struct NoMemory;

#[async_trait]
impl MemoryStore for NoMemory {
    async fn remember(&self, _item: MemoryItem) -> Result<()> {
        Ok(())
    }

    async fn recall(&self, _query: MemoryQuery) -> Result<Vec<MemoryItem>> {
        Ok(Vec::new())
    }
}
