//! Core-owned implementation of the Workflow host capability surface.
//!
//! Process Workflow delegates here, so module identity cannot change model,
//! tool, policy, cancellation, or event semantics.

mod model_stream;

use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use tokio::time::timeout;

use crate::{
    contracts::{
        CompactionInput, CompactionOutput, ContextBuildInput, ExecutionAttribution,
        ModelCallOrigin, ToolExposureInput, ToolExposureOutput, ToolExposureRequest,
        WorkflowInvocationContext, WorkflowRuntimeStatus,
    },
    domain::{AgentTask, Event, ToolCall, ToolResult, ToolSpec},
    model_standard::{CanonicalModelRequest, CanonicalModelResponse},
};

use super::{
    RuntimeCompactionHost, ToolOrchestrator,
    agent_control::{TASK_TOOL, calls_are_parallel_eligible},
    model_call_scope::with_model_call_origin,
};

/// Async host capability surface shared by all process Workflow exports.
pub(crate) struct WorkflowHostRuntime {
    ctx: WorkflowInvocationContext,
    tool_orchestrator: ToolOrchestrator,
    model_streams: model_stream::ModelStreams,
}

impl WorkflowHostRuntime {
    pub(crate) async fn checkpoint_history(
        &self,
        checkpoint: crate::contracts::WorkflowHistoryCheckpoint,
    ) -> Result<()> {
        self.ensure_active()?;
        self.ctx
            .agent()?
            .history_recorder
            .checkpoint(checkpoint)
            .await
    }
    pub(crate) fn new(ctx: WorkflowInvocationContext) -> Self {
        Self {
            ctx,
            model_streams: Default::default(),
            tool_orchestrator: ToolOrchestrator::default(),
        }
    }

    pub(crate) fn status(&self) -> WorkflowRuntimeStatus {
        WorkflowRuntimeStatus {
            cancelled: self.ctx.execution().is_cancelled(),
            queued_user_messages: self
                .ctx
                .agent()
                .map(|ctx| ctx.queued_user_messages())
                .unwrap_or(0)
                .min(u32::MAX as usize) as u32,
        }
    }

    pub(crate) async fn build_context(
        &self,
        task: AgentTask,
    ) -> Result<crate::domain::ContextBundle> {
        let ctx = self.ctx.clone();
        self.run_active(async move {
            let execution = ctx.execution();
            let attribution = match ctx.agent() {
                Ok(agent) => ExecutionAttribution::for_turn(
                    execution.scope.execution_id,
                    agent.session_id,
                    agent.thread_id,
                    agent.turn_id,
                ),
                Err(_) => ExecutionAttribution::detached(execution.scope.execution_id),
            };
            let timeout_ms = ctx.context_timeout_ms();

            timeout(
                Duration::from_millis(timeout_ms),
                ctx.context().build(ContextBuildInput {
                    task,
                    scope: execution.scope.clone(),
                    attribution,
                }),
            )
            .await
            .map_err(|_| anyhow!("context build timed out after {}ms", timeout_ms))?
        })
        .await
    }

    pub(crate) async fn start_model_stream(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<crate::contracts::WorkflowModelStreamCursor> {
        self.ensure_active()?;
        self.model_streams
            .start(self.ctx.execution(), request)
            .await
    }

    pub(crate) async fn next_model_stream(
        &self,
        cursor: crate::contracts::WorkflowModelStreamCursor,
    ) -> Result<crate::contracts::WorkflowModelStreamItem> {
        self.run_active(self.model_streams.next(cursor)).await
    }

    pub(crate) async fn close_model_stream(&self) -> bool {
        self.model_streams.close().await
    }

    pub(crate) async fn complete_model(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse> {
        let ctx = self.ctx.clone();
        self.run_active(async move {
            with_model_call_origin(
                ModelCallOrigin::Direct,
                ctx.execution().require_model()?.complete(request),
            )
            .await
        })
        .await
    }

    pub(crate) async fn compact_history(&self, input: CompactionInput) -> Result<CompactionOutput> {
        let ctx = self.ctx.agent()?.clone();
        self.run_active(async move {
            ctx.emit(Event::HistoryCompactionStarted {
                reason: input.reason.clone(),
                input_messages: input.request.messages.len(),
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
                        input_messages: input.request.messages.len(),
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
        match self.ctx.agent() {
            Ok(ctx) => Ok(self.tool_orchestrator.visible_tool_specs(ctx, &cwd)),
            Err(_) => Ok(self.detached_tools().visible_specs(&cwd)),
        }
    }

    pub(crate) async fn select_tools(
        &self,
        request: ToolExposureRequest,
    ) -> Result<ToolExposureOutput> {
        let candidates = self.visible_tools(request.cwd.clone())?;
        let ctx = self.ctx.clone();
        self.run_active(async move {
            let output = ctx
                .tool_exposure()
                .select(ToolExposureInput::new(request, candidates.clone()))
                .await?;
            output.validate_against(&candidates)?;
            Ok(output)
        })
        .await
    }

    pub(crate) async fn execute_tool(&self, task: AgentTask, call: ToolCall) -> Result<ToolResult> {
        self.run_active(async {
            match self.ctx.agent() {
                Ok(ctx) => self.tool_orchestrator.execute(ctx, &task, call).await,
                Err(_) => self.detached_tools().execute(task.cwd, call).await,
            }
        })
        .await
    }

    pub(crate) async fn execute_tools(
        &self,
        task: AgentTask,
        calls: Vec<ToolCall>,
    ) -> Result<Vec<ToolResult>> {
        self.run_active(execute_tool_batch(self, &task, calls))
            .await
    }

    pub(crate) async fn emit_event(&self, event: Event) -> Result<()> {
        let ctx = self.ctx.agent()?.clone();
        self.run_active(async move { ctx.emit(event).await }).await
    }

    fn detached_tools(&self) -> super::BoundTools {
        let ctx = self.ctx.execution();
        super::BoundTools::new(
            ctx.tools.clone(),
            ctx.policy.clone(),
            ctx.approval.clone(),
            ctx.permission_grants.clone(),
            super::ToolExecutionBinding::detached(ctx.scope.clone()),
        )
        .with_hooks(ctx.hooks.clone())
    }

    fn ensure_active(&self) -> Result<()> {
        if self.ctx.execution().is_cancelled() {
            return Err(anyhow!("turn canceled by client"));
        }
        Ok(())
    }

    async fn run_active<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        let cancellation = self.ctx.execution().scope.cancellation.clone();
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
/// workflows. Consecutive explicitly parallel calls may overlap; other calls
/// fence the sequence. The root-owned task group has its own role eligibility.
async fn execute_tool_batch(
    host: &WorkflowHostRuntime,
    task: &AgentTask,
    calls: Vec<ToolCall>,
) -> Result<Vec<ToolResult>> {
    let specs = host.visible_tools(task.cwd.clone())?;
    let parallel = |call: &ToolCall| {
        specs
            .iter()
            .find(|spec| spec.name == call.name)
            .is_some_and(|spec| spec.supports_parallel_tool_calls)
    };

    let mut results = Vec::with_capacity(calls.len());
    let mut queue = calls.into_iter().peekable();
    while let Some(call) = queue.next() {
        if call.name == TASK_TOOL {
            let mut group = vec![call];
            while queue.peek().is_some_and(|call| call.name == TASK_TOOL) {
                group.push(queue.next().expect("peeked task call"));
            }
            if calls_are_parallel_eligible(
                &group,
                &host
                    .ctx
                    .agent()
                    .ok()
                    .and_then(|ctx| ctx.agent_control.as_ref())
                    .map(|control| control.profiles())
                    .unwrap_or_default(),
            ) {
                let outputs = futures_util::future::join_all(
                    group
                        .into_iter()
                        .map(|call| host.execute_tool(task.clone(), call)),
                )
                .await;
                for output in outputs {
                    results.push(output?);
                }
            } else {
                for call in group {
                    results.push(host.execute_tool(task.clone(), call).await?);
                }
            }
            continue;
        }
        if parallel(&call) {
            let mut group = vec![call];
            while queue.peek().is_some_and(&parallel) {
                group.push(queue.next().expect("peeked call"));
            }
            let outputs = futures_util::future::join_all(
                group
                    .into_iter()
                    .map(|call| host.execute_tool(task.clone(), call)),
            )
            .await;
            for output in outputs {
                results.push(output?);
            }
        } else {
            results.push(host.execute_tool(task.clone(), call).await?);
        }
    }
    Ok(results)
}
