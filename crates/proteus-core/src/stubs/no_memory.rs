use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{MemoryInvocationContext, MemoryStore},
    domain::{MemoryItem, MemoryQuery},
};

#[derive(Debug)]
pub struct NoMemory;

#[async_trait]
impl MemoryStore for NoMemory {
    async fn remember(&self, _item: MemoryItem, _ctx: MemoryInvocationContext) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: MemoryQuery,
        _ctx: MemoryInvocationContext,
    ) -> Result<Vec<MemoryItem>> {
        Ok(Vec::new())
    }
}
