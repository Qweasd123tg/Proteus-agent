use anyhow::Result;
use async_trait::async_trait;

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
            return Err(interrupted());
        }
        let ctx = self.ctx.clone();
        let cancellation = ctx.execution.scope.cancellation.clone();
        tokio::select! {
            result = without_root_steering(ctx.execution.model.complete(request)) => result,
            _ = cancellation.cancelled() => Err(interrupted()),
        }
    }
}

fn interrupted() -> anyhow::Error {
    crate::model_standard::ModelFailure::new(
        crate::model_standard::ModelFailureKind::Interrupted,
        "turn canceled by client",
    )
    .into()
}
