use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::time::timeout;

use crate::{
    contracts::{AgentWorkflowContext, CompactionHost},
    model_standard::{CanonicalModelRequest, CanonicalModelResponse},
};

use super::without_root_steering;

/// Host-owned implementation of the capabilities available to every
/// `HistoryCompactor` invocation, independent of module identity.
#[derive(Clone)]
pub struct RuntimeCompactionHost {
    ctx: AgentWorkflowContext,
}

impl RuntimeCompactionHost {
    pub fn new(ctx: AgentWorkflowContext) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl CompactionHost for RuntimeCompactionHost {
    fn is_cancelled(&self) -> bool {
        self.ctx.is_cancelled()
    }

    async fn complete_model(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse> {
        if self.ctx.is_cancelled() {
            anyhow::bail!("turn canceled by client");
        }
        let ctx = self.ctx.clone();
        let cancellation = ctx.execution.scope.cancellation.clone();
        tokio::select! {
            result = async move {
                let completion = without_root_steering(ctx.execution.model.complete(request));
                if ctx.execution.model_timeout_ms == 0 {
                    completion.await
                } else {
                    timeout(Duration::from_millis(ctx.execution.model_timeout_ms), completion)
                        .await
                        .map_err(|_| anyhow!("model request timed out after {}ms", ctx.execution.model_timeout_ms))?
                }
            } => result,
            _ = cancellation.cancelled() => Err(anyhow!("turn canceled by client")),
        }
    }
}
