use std::sync::Arc;

use anyhow::Result;

use crate::{
    contracts::{ExecutionAttribution, ExecutionScope, MemoryInvocationContext, MemoryStore},
    domain::{MemoryItem, MemoryQuery},
};

/// Memory store bound immutably to one logical execution.
///
/// The selected store, execution identity and cancellation token cannot drift
/// between admission and invocation. Callers receive no raw registry access.
#[derive(Clone)]
pub struct BoundMemory {
    store: Arc<dyn MemoryStore>,
    context: MemoryInvocationContext,
}

impl BoundMemory {
    pub fn detached(store: Arc<dyn MemoryStore>, scope: ExecutionScope) -> Self {
        let attribution = ExecutionAttribution::detached(scope.execution_id);
        Self::attributed(store, scope, attribution)
    }

    pub(crate) fn attributed(
        store: Arc<dyn MemoryStore>,
        scope: ExecutionScope,
        attribution: ExecutionAttribution,
    ) -> Self {
        debug_assert_eq!(scope.execution_id, attribution.execution_id);
        Self {
            store,
            context: MemoryInvocationContext::new(attribution, scope.cancellation),
        }
    }

    pub fn context(&self) -> &MemoryInvocationContext {
        &self.context
    }

    pub async fn remember(&self, item: MemoryItem) -> Result<()> {
        if self.context.cancellation.is_cancelled() {
            anyhow::bail!("memory execution canceled");
        }
        self.store.remember(item, self.context.clone()).await
    }

    pub async fn recall(&self, query: MemoryQuery) -> Result<Vec<MemoryItem>> {
        if self.context.cancellation.is_cancelled() {
            anyhow::bail!("memory execution canceled");
        }
        self.store.recall(query, self.context.clone()).await
    }
}
