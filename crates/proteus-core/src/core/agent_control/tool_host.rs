//! Per-invocation binding from model-facing tools to the current runtime.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;

use crate::{
    contracts::{
        AgentControlHandle, AgentControlMessage, AgentControlRequest, AgentControlResult,
        AgentControlToolHost, AgentWorkflowContext, CancellationToken,
    },
    domain::SessionId,
};

pub(super) fn bind(
    ctx: &AgentWorkflowContext,
    cancellation: CancellationToken,
) -> Option<Arc<dyn AgentControlToolHost>> {
    ctx.agent_control.as_ref()?;
    Some(Arc::new(RuntimeAgentControlToolHost {
        ctx: ctx.clone().with_cancellation(cancellation),
    }))
}

/// The adapter never constructs a runtime context: each instance is bound to
/// the caller's session/thread/turn and per-tool cancellation token.
struct RuntimeAgentControlToolHost {
    ctx: AgentWorkflowContext,
}

#[async_trait]
impl AgentControlToolHost for RuntimeAgentControlToolHost {
    fn session_id(&self) -> Option<SessionId> {
        Some(self.ctx.session_id)
    }

    async fn run_agent(&self, request: AgentControlRequest) -> Result<AgentControlResult> {
        self.service()?.run(request, self.ctx.clone()).await
    }

    async fn spawn_agent(&self, request: AgentControlRequest) -> Result<AgentControlHandle> {
        self.service()?.spawn(request, self.ctx.clone()).await
    }

    async fn wait_agent(&self, handle: &AgentControlHandle) -> Result<AgentControlResult> {
        self.service()?.wait(handle).await
    }

    async fn cancel_agent(&self, handle: &AgentControlHandle) -> Result<()> {
        self.service()?.cancel(handle).await
    }

    async fn send_agent(
        &self,
        handle: &AgentControlHandle,
        message: AgentControlMessage,
    ) -> Result<()> {
        self.service()?.send(handle, message).await
    }
}

impl RuntimeAgentControlToolHost {
    fn service(&self) -> Result<&Arc<dyn crate::contracts::AgentControl>> {
        self.ctx
            .agent_control
            .as_ref()
            .ok_or_else(|| anyhow!("agent control is disabled"))
    }
}
