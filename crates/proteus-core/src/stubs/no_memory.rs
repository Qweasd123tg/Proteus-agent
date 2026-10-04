use anyhow::{Result, bail};
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
        bail!("memory is not configured; select modules.memory before remembering facts")
    }

    async fn recall(
        &self,
        _query: MemoryQuery,
        _ctx: MemoryInvocationContext,
    ) -> Result<Vec<MemoryItem>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        contracts::{ExecutionAttribution, Tool, ToolContext},
        domain::{ToolCall, new_call_id, new_execution_id},
        tools::RememberFactTool,
    };
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn absent_memory_rejects_explicit_write_and_remember_fact() {
        let attribution = ExecutionAttribution::detached(new_execution_id());
        let ctx = MemoryInvocationContext::new(attribution.clone(), Default::default());
        let memory = Arc::new(NoMemory);
        let error = memory
            .remember(
                MemoryItem::new("fact", "durable fact", json!({})),
                ctx.clone(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("memory is not configured"));
        assert!(
            memory
                .recall(MemoryQuery::new("durable", 10), ctx)
                .await
                .unwrap()
                .is_empty()
        );
        let tool = RememberFactTool::new(memory);
        let error = tool
            .invoke(
                &ToolCall::new(
                    new_call_id(),
                    "remember_fact",
                    json!({"kind":"fact", "content":"durable fact"}),
                ),
                ToolContext::new(std::env::temp_dir(), attribution),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("memory is not configured"));
    }
}
