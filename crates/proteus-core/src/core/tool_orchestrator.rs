use std::{path::Path, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{AgentWorkflowContext, ExecutionAttribution, RequestOrigin, ToolContext},
    core::{AttributedUserInputTransport, BoundTools, ToolExecutionBinding, ToolExecutionObserver},
    domain::{AgentTask, Event, ToolCall, ToolResult, ToolSpec},
};

const DEFAULT_MAX_OUTPUT_BYTES: usize = 200_000;

/// Agent-layer adapter around the generic execution-bound tool mechanism.
#[derive(Debug, Clone)]
pub struct ToolOrchestrator {
    default_timeout_ms: u64,
    max_output_bytes: usize,
}

impl Default for ToolOrchestrator {
    fn default() -> Self {
        Self {
            default_timeout_ms: 30_000,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

impl ToolOrchestrator {
    pub fn visible_tool_specs(&self, ctx: &AgentWorkflowContext, cwd: &Path) -> Vec<ToolSpec> {
        self.bind(ctx).visible_specs(cwd)
    }

    pub async fn execute(
        &self,
        ctx: &AgentWorkflowContext,
        task: &AgentTask,
        call: ToolCall,
    ) -> Result<ToolResult> {
        if ctx.is_cancelled() {
            anyhow::bail!("turn canceled by client");
        }

        let bound = self.bind(ctx);
        let observer = AgentToolExecutionObserver { ctx: ctx.clone() };
        let origin = request_origin(ctx);
        bound
            .execute_enriched(task.cwd.clone(), call, &observer, |tool_ctx| {
                enrich_agent_tool_context(ctx, task, origin, tool_ctx);
            })
            .await
    }

    fn bind(&self, ctx: &AgentWorkflowContext) -> BoundTools {
        let binding = ToolExecutionBinding::attributed(
            ctx.execution.scope.clone(),
            tool_attribution(ctx),
            request_origin(ctx),
            ctx.tool_recorder.clone(),
        );
        BoundTools::new(
            ctx.execution.tools.clone(),
            ctx.execution.policy.clone(),
            ctx.execution.approval.clone(),
            ctx.execution.permission_grants.clone(),
            binding,
        )
        .with_limits(self.default_timeout_ms, self.max_output_bytes)
    }
}

fn enrich_agent_tool_context(
    ctx: &AgentWorkflowContext,
    task: &AgentTask,
    origin: RequestOrigin,
    tool_ctx: &mut ToolContext,
) {
    tool_ctx.user_input = Some(Arc::new(AttributedUserInputTransport::new(
        ctx.user_input.clone(),
        origin,
    )));
    tool_ctx.task = Some(task.clone());
    tool_ctx.agent_control =
        crate::core::agent_control::bind_tool_host(ctx, tool_ctx.cancellation.clone());
}

fn request_origin(ctx: &AgentWorkflowContext) -> RequestOrigin {
    let origin =
        RequestOrigin::for_turn(ctx.execution.scope.execution_id, ctx.thread_id, ctx.turn_id);
    match &ctx.thread_label {
        Some(label) => origin.with_label(label.clone()),
        None => origin,
    }
}

fn tool_attribution(ctx: &AgentWorkflowContext) -> ExecutionAttribution {
    ExecutionAttribution::for_turn(
        ctx.execution.scope.execution_id,
        ctx.session_id,
        ctx.thread_id,
        ctx.turn_id,
    )
}

struct AgentToolExecutionObserver {
    ctx: AgentWorkflowContext,
}

#[async_trait]
impl ToolExecutionObserver for AgentToolExecutionObserver {
    async fn tool_call_requested(&self, call: &ToolCall) -> Result<()> {
        self.ctx
            .emit(Event::ToolCallRequested { call: call.clone() })
            .await
    }

    async fn approval_requested(&self, call: &ToolCall, reason: &str) -> Result<()> {
        self.ctx
            .emit(Event::ApprovalRequested {
                call_id: call.id.clone(),
                reason: reason.to_owned(),
            })
            .await
    }

    async fn approval_resolved(&self, call: &ToolCall, approved: bool) -> Result<()> {
        self.ctx
            .emit(Event::ApprovalResolved {
                call_id: call.id.clone(),
                approved,
            })
            .await
    }

    async fn tool_finished(&self, result: &ToolResult) -> Result<()> {
        self.ctx
            .emit(Event::ToolFinished {
                result: result.clone(),
            })
            .await
    }
}
