use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{CancellationToken, ExecutionAttribution},
    domain::{MemoryItem, MemoryQuery},
};

/// Immutable host-owned context for one memory invocation.
///
/// Attribution crosses a process boundary as part of `memory/v2`; the
/// cancellation token stays in the host and drives targeted broker cancel.
#[derive(Clone)]
pub struct MemoryInvocationContext {
    pub attribution: ExecutionAttribution,
    pub cancellation: CancellationToken,
}

impl MemoryInvocationContext {
    pub fn new(attribution: ExecutionAttribution, cancellation: CancellationToken) -> Self {
        Self {
            attribution,
            cancellation,
        }
    }
}

#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn remember(&self, item: MemoryItem, ctx: MemoryInvocationContext) -> Result<()>;
    async fn recall(
        &self,
        query: MemoryQuery,
        ctx: MemoryInvocationContext,
    ) -> Result<Vec<MemoryItem>>;
}
