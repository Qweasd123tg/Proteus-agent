//! Core-owned implementation of the Workflow host capability surface.
//!
//! Process Workflow v1 delegates here, so module identity cannot change model,
//! tool, policy, cancellation, or event semantics.

use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use tokio::time::timeout;

use crate::{
    contracts::{
        CompactionInput, CompactionOutput, ContextBuildInput, RuntimeContext, ToolExposureInput,
        ToolExposureOutput, ToolExposureRequest, WorkflowRuntimeStatus,
    },
    domain::{AgentTask, Event, ToolCall, ToolResult, ToolSpec},
    model_standard::{CanonicalModelRequest, CanonicalModelResponse},
    tools::{TASK_TOOL, calls_are_parallel_eligible},
};

use super::{RuntimeCompactionHost, ToolOrchestrator};

/// Async host capability surface shared by all process Workflow exports.
pub(crate) struct WorkflowHostRuntime {
    ctx: RuntimeContext,
    tool_orchestrator: ToolOrchestrator,
}

impl WorkflowHostRuntime {
    pub(crate) fn new(ctx: RuntimeContext) -> Self {
        Self {
            ctx,
            tool_orchestrator: ToolOrchestrator::default(),
        }
    }

    pub(crate) fn status(&self) -> WorkflowRuntimeStatus {
        WorkflowRuntimeStatus {
            cancelled: self.ctx.is_cancelled(),
            queued_user_messages: self.ctx.queued_user_messages().min(u32::MAX as usize) as u32,
        }
    }

    pub(crate) async fn build_context(
        &self,
        task: AgentTask,
    ) -> Result<crate::domain::ContextBundle> {
        let ctx = self.ctx.clone();
        self.run_active(async move {
            timeout(
                Duration::from_millis(ctx.context_timeout_ms),
                ctx.context.build(ContextBuildInput {
                    task,
                    search: ctx.search.clone(),
                    memory: ctx.memory.clone(),
                }),
            )
            .await
            .map_err(|_| anyhow!("context build timed out after {}ms", ctx.context_timeout_ms))?
        })
        .await
    }

    pub(crate) async fn complete_model(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse> {
        let ctx = self.ctx.clone();
        self.run_active(async move {
            if ctx.model_timeout_ms == 0 {
                ctx.model.complete(request).await
            } else {
                timeout(
                    Duration::from_millis(ctx.model_timeout_ms),
                    ctx.model.complete(request),
                )
                .await
                .map_err(|_| anyhow!("model request timed out after {}ms", ctx.model_timeout_ms))?
            }
        })
        .await
    }

    pub(crate) async fn compact_history(&self, input: CompactionInput) -> Result<CompactionOutput> {
        let ctx = self.ctx.clone();
        self.run_active(async move {
            ctx.emit(Event::HistoryCompactionStarted {
                reason: input.reason.clone(),
                input_messages: input.messages.len(),
                token_estimate: input.token_estimate,
                trigger_tokens: None,
            })
            .await?;
            let host = Arc::new(RuntimeCompactionHost::new(ctx.clone()));
            match ctx.compactor.compact(input.clone(), host).await {
                Ok(output) => {
                    let report = crate::domain::HistoryCompactionReport::from_compaction_output(
                        &input, &output,
                    );
                    ctx.emit(Event::HistoryCompactionCompleted {
                        report: report.clone(),
                    })
                    .await?;
                    Ok(output)
                }
                Err(error) => {
                    ctx.emit(Event::HistoryCompactionFailed {
                        reason: input.reason.clone(),
                        input_messages: input.messages.len(),
                        token_estimate: input.token_estimate,
                        trigger_tokens: None,
                        message: format!("{error:#}"),
                    })
                    .await?;
                    Err(error)
                }
            }
        })
        .await
    }

    pub(crate) fn visible_tools(&self, cwd: PathBuf) -> Result<Vec<ToolSpec>> {
        self.ensure_active()?;
        Ok(self.tool_orchestrator.visible_tool_specs(&self.ctx, &cwd))
    }

    pub(crate) async fn select_tools(
        &self,
        request: ToolExposureRequest,
    ) -> Result<ToolExposureOutput> {
        let candidates = self
            .tool_orchestrator
            .visible_tool_specs(&self.ctx, &request.cwd);
        let ctx = self.ctx.clone();
        self.run_active(async move {
            ctx.tool_exposure
                .select(ToolExposureInput::new(request, candidates))
                .await
        })
        .await
    }

    pub(crate) async fn execute_tool(&self, task: AgentTask, call: ToolCall) -> Result<ToolResult> {
        let ctx = self.ctx.clone();
        let orchestrator = self.tool_orchestrator.clone();
        self.run_active(async move { orchestrator.execute(&ctx, &task, call).await })
            .await
    }

    pub(crate) async fn execute_tools(
        &self,
        task: AgentTask,
        calls: Vec<ToolCall>,
    ) -> Result<Vec<ToolResult>> {
        let ctx = self.ctx.clone();
        let orchestrator = self.tool_orchestrator.clone();
        self.run_active(async move { execute_tool_batch(&orchestrator, &ctx, &task, calls).await })
            .await
    }

    pub(crate) async fn emit_event(&self, event: Event) -> Result<()> {
        let ctx = self.ctx.clone();
        self.run_active(async move { ctx.emit(event).await }).await
    }

    fn ensure_active(&self) -> Result<()> {
        if self.ctx.is_cancelled() {
            return Err(anyhow!("turn canceled by client"));
        }
        Ok(())
    }

    async fn run_active<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        let cancellation = self.ctx.cancellation.clone();
        if cancellation.is_cancelled() {
            return Err(anyhow!("turn canceled by client"));
        }
        tokio::select! {
            result = future => result,
            _ = cancellation.cancelled() => Err(anyhow!("turn canceled by client")),
        }
    }
}

/// Executes a batch through the same registry/policy/safety path as in-process
/// workflows. Consecutive read-only calls may run concurrently; mutations keep
/// model order.
async fn execute_tool_batch(
    orchestrator: &ToolOrchestrator,
    ctx: &RuntimeContext,
    task: &AgentTask,
    calls: Vec<ToolCall>,
) -> Result<Vec<ToolResult>> {
    use crate::domain::ToolSafety;

    let specs = orchestrator.visible_tool_specs(ctx, &task.cwd);
    let read_only = |call: &ToolCall| {
        specs
            .iter()
            .find(|spec| spec.name == call.name)
            .is_some_and(|spec| matches!(spec.safety, ToolSafety::ReadOnly))
    };

    let mut results = Vec::with_capacity(calls.len());
    let mut queue = calls.into_iter().peekable();
    while let Some(call) = queue.next() {
        if call.name == TASK_TOOL {
            let mut group = vec![call];
            while queue.peek().is_some_and(|call| call.name == TASK_TOOL) {
                group.push(queue.next().expect("peeked task call"));
            }
            if calls_are_parallel_eligible(&group, &ctx.subagent.roles()) {
                let outputs = futures_util::future::join_all(
                    group
                        .into_iter()
                        .map(|call| orchestrator.execute(ctx, task, call)),
                )
                .await;
                for output in outputs {
                    results.push(output?);
                }
            } else {
                for call in group {
                    results.push(orchestrator.execute(ctx, task, call).await?);
                }
            }
            continue;
        }
        if read_only(&call) {
            let mut group = vec![call];
            while queue.peek().is_some_and(&read_only) {
                group.push(queue.next().expect("peeked call"));
            }
            let outputs = futures_util::future::join_all(
                group
                    .into_iter()
                    .map(|call| orchestrator.execute(ctx, task, call)),
            )
            .await;
            for output in outputs {
                results.push(output?);
            }
        } else {
            results.push(orchestrator.execute(ctx, task, call).await?);
        }
    }
    Ok(results)
}
