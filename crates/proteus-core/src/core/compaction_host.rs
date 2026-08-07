use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::time::timeout;

use crate::{
    contracts::{CompactionHost, RuntimeContext},
    model_standard::{CanonicalModelRequest, CanonicalModelResponse},
};

use super::without_root_steering;

/// Host-owned implementation of the capabilities available to every
/// `HistoryCompactor` invocation, independent of module identity.
#[derive(Clone)]
pub struct RuntimeCompactionHost {
    ctx: RuntimeContext,
}

impl RuntimeCompactionHost {
    pub fn new(ctx: RuntimeContext) -> Self {
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
        let cancellation = ctx.cancellation.clone();
        tokio::select! {
            result = async move {
                let completion = without_root_steering(ctx.model.complete(request));
                if ctx.model_timeout_ms == 0 {
                    completion.await
                } else {
                    timeout(Duration::from_millis(ctx.model_timeout_ms), completion)
                        .await
                        .map_err(|_| anyhow!("model request timed out after {}ms", ctx.model_timeout_ms))?
                }
            } => result,
            _ = cancellation.cancelled() => Err(anyhow!("turn canceled by client")),
        }
    }
}
