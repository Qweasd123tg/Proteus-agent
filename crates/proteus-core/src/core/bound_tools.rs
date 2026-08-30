use std::{
    future,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::time::sleep;

use crate::{
    contracts::{
        ApprovalPolicy, ApprovalRequest, ApprovalTransport, ExecutionAttribution,
        ExecutionPermissionGrants, ExecutionScope, NoopToolExecutionRecorder, PolicyContext,
        PolicyVisibilityContext, RequestOrigin, ToolContext, ToolExecutionRecorder, ToolRegistry,
    },
    domain::{PolicyDecision, ToolCall, ToolCallResolution, ToolResult, ToolSpec},
};

use self::support::{
    intercept_apply_patch_call, metadata_with, truncate_utf8, validate_tool_call_args,
    visibility_decision_allows,
};

#[cfg(test)]
use self::support::extract_apply_patch_body;

mod support;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 200_000;

/// Immutable execution binding for the tool capability.
///
/// Detached execution carries only its execution identity. An upper layer may
/// add an optional presentation projection without putting it into the scope.
#[derive(Clone)]
pub struct ToolExecutionBinding {
    scope: ExecutionScope,
    attribution: ExecutionAttribution,
    origin: RequestOrigin,
    recorder: Arc<dyn ToolExecutionRecorder>,
}

impl ToolExecutionBinding {
    pub fn detached(scope: ExecutionScope) -> Self {
        let execution_id = scope.execution_id;
        Self {
            scope,
            attribution: ExecutionAttribution::detached(execution_id),
            origin: RequestOrigin::for_execution(execution_id),
            recorder: Arc::new(NoopToolExecutionRecorder),
        }
    }

    pub fn with_recorder(mut self, recorder: Arc<dyn ToolExecutionRecorder>) -> Self {
        self.recorder = recorder;
        self
    }

    pub(crate) fn attributed(
        scope: ExecutionScope,
        attribution: ExecutionAttribution,
        origin: RequestOrigin,
        recorder: Arc<dyn ToolExecutionRecorder>,
    ) -> Self {
        debug_assert_eq!(scope.execution_id, attribution.execution_id);
        debug_assert_eq!(scope.execution_id, origin.execution_id);
        debug_assert_eq!(
            attribution.agent.map(|agent| agent.thread_id),
            origin.thread_id
        );
        debug_assert_eq!(attribution.agent.map(|agent| agent.turn_id), origin.turn_id);
        Self {
            scope,
            attribution,
            origin,
            recorder,
        }
    }

    pub fn scope(&self) -> &ExecutionScope {
        &self.scope
    }

    pub fn attribution(&self) -> ExecutionAttribution {
        self.attribution
    }
}

/// Tool registry and safety path bound immutably to one execution.
///
/// The public execution API needs only a workspace and a canonical call.
/// Agent presentation and control-plane enrichment are supplied by the thin
/// adapter in `tool_orchestrator` and are not prerequisites for this handle.
#[derive(Clone)]
pub struct BoundTools {
    registry: ToolRegistry,
    policy: Arc<dyn ApprovalPolicy>,
    approval: Arc<dyn ApprovalTransport>,
    permission_grants: Arc<ExecutionPermissionGrants>,
    binding: ToolExecutionBinding,
    default_timeout_ms: u64,
    max_output_bytes: usize,
}

impl BoundTools {
    pub fn new(
        registry: ToolRegistry,
        policy: Arc<dyn ApprovalPolicy>,
        approval: Arc<dyn ApprovalTransport>,
        permission_grants: Arc<ExecutionPermissionGrants>,
        binding: ToolExecutionBinding,
    ) -> Self {
        Self {
            registry,
            policy,
            approval,
            permission_grants,
            binding,
            default_timeout_ms: DEFAULT_TIMEOUT_MS,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    pub(crate) fn with_limits(mut self, default_timeout_ms: u64, max_output_bytes: usize) -> Self {
        self.default_timeout_ms = default_timeout_ms;
        self.max_output_bytes = max_output_bytes;
        self
    }

    pub fn binding(&self) -> &ToolExecutionBinding {
        &self.binding
    }

    pub fn visible_specs(&self, cwd: &Path) -> Vec<ToolSpec> {
        self.registry
            .specs()
            .into_iter()
            .filter(|spec| {
                visibility_decision_allows(
                    spec,
                    self.policy
                        .evaluate_visibility(&PolicyVisibilityContext::new(
                            cwd.to_path_buf(),
                            spec.clone(),
                        )),
                    self.approval.can_request_approval(),
                )
            })
            .collect()
    }

    pub async fn execute(&self, cwd: PathBuf, call: ToolCall) -> Result<ToolResult> {
        self.execute_enriched(cwd, call, &NoopToolExecutionObserver, |_| {})
            .await
    }

    pub(crate) async fn execute_enriched<F>(
        &self,
        cwd: PathBuf,
        mut call: ToolCall,
        observer: &dyn ToolExecutionObserver,
        enrich: F,
    ) -> Result<ToolResult>
    where
        F: FnOnce(&mut ToolContext),
    {
        if self.binding.scope.cancellation.is_cancelled() {
            anyhow::bail!("tool execution canceled");
        }

        if let Some(raw_arguments) = call.raw_arguments.as_deref()
            && let Ok(parsed_arguments) = serde_json::from_str(raw_arguments)
        {
            call.args = parsed_arguments;
        }
        let call = intercept_apply_patch_call(&self.registry, &call).unwrap_or(call);

        self.binding
            .recorder
            .tool_call_requested(self.binding.attribution, &call)
            .await?;
        observer.tool_call_requested(&call).await?;

        let tool_spec = self.registry.spec(&call.name).ok();
        if let Some(spec) = tool_spec.as_ref()
            && let Some(error) = validate_tool_call_args(&call, spec)
        {
            self.record_resolution(
                &call,
                &ToolCallResolution::ValidationFailed {
                    reason: error.clone(),
                },
            )
            .await?;
            let result = ToolResult::error(call.id.clone(), error).with_metadata(json!({
                "tool": call.name,
                "validation_error": true,
            }));
            return self.finish(observer, result).await;
        }

        let decision = self.evaluate_access(&cwd, &call, tool_spec.clone());
        let mut enrich = Some(enrich);
        match decision {
            PolicyDecision::Allow => {
                self.record_resolution(&call, &ToolCallResolution::Allowed)
                    .await?;
                self.invoke_allowed(
                    observer,
                    &call,
                    tool_spec,
                    cwd,
                    enrich.take().expect("tool context enrichment"),
                )
                .await
            }
            PolicyDecision::Ask { reason } => {
                self.binding
                    .recorder
                    .tool_approval_requested(self.binding.attribution, &call, &reason)
                    .await?;
                observer.approval_requested(&call, &reason).await?;
                let approval_request = self.approval.request_approval(
                    ApprovalRequest::new(
                        call.clone(),
                        cwd.clone(),
                        reason.clone(),
                        tool_spec.clone(),
                    )
                    .with_origin(self.binding.origin.clone()),
                );
                let approval = tokio::select! {
                    result = approval_request => result?,
                    _ = self.binding.scope.cancellation.cancelled() => {
                        return Err(anyhow!("tool execution canceled"));
                    }
                };
                observer.approval_resolved(&call, approval.approved).await?;
                if approval.approved {
                    self.record_resolution(&call, &ToolCallResolution::Approved)
                        .await?;
                    let result = self
                        .invoke_allowed(
                            observer,
                            &call,
                            tool_spec,
                            cwd,
                            enrich.take().expect("tool context enrichment"),
                        )
                        .await?;
                    self.merge_granted_permissions(&result);
                    return Ok(result);
                }

                let result = ToolResult::error(
                    call.id.clone(),
                    approval
                        .note
                        .unwrap_or_else(|| format!("tool call was not approved: {reason}")),
                );
                self.record_resolution(
                    &call,
                    &ToolCallResolution::ApprovalDenied {
                        reason: result.text_or_status(),
                    },
                )
                .await?;
                self.finish(observer, result).await
            }
            PolicyDecision::Deny { reason } => {
                self.record_resolution(
                    &call,
                    &ToolCallResolution::PolicyDenied {
                        reason: reason.clone(),
                    },
                )
                .await?;
                self.finish(observer, ToolResult::error(call.id.clone(), reason))
                    .await
            }
            other => {
                let reason = format!("unsupported policy decision: {other:?}");
                self.record_resolution(
                    &call,
                    &ToolCallResolution::Unsupported {
                        reason: reason.clone(),
                    },
                )
                .await?;
                self.finish(observer, ToolResult::error(call.id.clone(), reason))
                    .await
            }
        }
    }

    fn evaluate_access(
        &self,
        cwd: &Path,
        call: &ToolCall,
        tool_spec: Option<ToolSpec>,
    ) -> PolicyDecision {
        let Some(spec) = tool_spec else {
            return PolicyDecision::Deny {
                reason: format!("unknown tool: {}", call.name),
            };
        };
        self.policy.evaluate(
            call,
            &PolicyContext::new(cwd.to_path_buf(), Some(spec))
                .with_granted_permissions(self.permission_grants.snapshot()),
        )
    }

    async fn record_resolution(
        &self,
        call: &ToolCall,
        resolution: &ToolCallResolution,
    ) -> Result<()> {
        self.binding
            .recorder
            .tool_call_resolved(self.binding.attribution, call, resolution)
            .await
    }

    async fn invoke_allowed<F>(
        &self,
        observer: &dyn ToolExecutionObserver,
        call: &ToolCall,
        tool_spec: Option<ToolSpec>,
        cwd: PathBuf,
        enrich: F,
    ) -> Result<ToolResult>
    where
        F: FnOnce(&mut ToolContext),
    {
        let tool = self
            .registry
            .get(&call.name)
            .ok_or_else(|| anyhow!("unknown tool: {}", call.name))?;
        let timeout_ms = tool_spec
            .as_ref()
            .and_then(|spec| spec.timeout_ms)
            .unwrap_or(self.default_timeout_ms);
        let started = Instant::now();
        let tool_cancellation = self.binding.scope.cancellation.child_token();
        let mut tool_ctx = ToolContext {
            cwd,
            attribution: self.binding.attribution,
            cancellation: tool_cancellation.clone(),
            user_input: None,
            task: None,
            agent_control: None,
        };
        enrich(&mut tool_ctx);
        // Binding-owned fields remain authoritative after optional enrichment.
        tool_ctx.attribution = self.binding.attribution;
        tool_ctx.cancellation = tool_cancellation.clone();
        let timeout_future = async move {
            if timeout_ms == 0 {
                future::pending::<()>().await;
            } else {
                sleep(Duration::from_millis(timeout_ms)).await;
            }
        };
        tokio::pin!(timeout_future);
        let result = tokio::select! {
            result = tool.invoke(call, tool_ctx) => {
                match result {
                    Ok(result) => result,
                    Err(error) => ToolResult::error(call.id.clone(), error.to_string())
                        .with_metadata(json!({ "tool": call.name })),
                }
            }
            _ = &mut timeout_future => {
                tool_cancellation.cancel();
                ToolResult::error(
                    call.id.clone(),
                    format!("tool timed out after {timeout_ms}ms"),
                )
                .with_metadata(json!({
                    "tool": call.name,
                    "timed_out": true,
                    "timeout_ms": timeout_ms,
                }))
            },
            _ = self.binding.scope.cancellation.cancelled() => {
                ToolResult::error(call.id.clone(), "tool call canceled")
                    .with_metadata(json!({
                        "tool": call.name,
                        "canceled": true,
                    }))
            }
        };

        let mut result = self.truncate_result(result);
        result.metadata = metadata_with(
            result.metadata,
            "duration_ms",
            json!(started.elapsed().as_millis() as u64),
        );
        self.finish(observer, result).await
    }

    async fn finish(
        &self,
        observer: &dyn ToolExecutionObserver,
        result: ToolResult,
    ) -> Result<ToolResult> {
        self.binding
            .recorder
            .tool_result_recorded(self.binding.attribution, &result)
            .await?;
        observer.tool_finished(&result).await?;
        Ok(result)
    }

    fn truncate_result(&self, mut result: ToolResult) -> ToolResult {
        let (output, output_truncated, output_original_bytes) =
            truncate_utf8(result.output, self.max_output_bytes, "output");
        result.output = output;

        let (error, error_truncated, error_original_bytes) = result
            .error
            .map(|error| truncate_utf8(error, self.max_output_bytes, "error"))
            .map(|(error, truncated, original_bytes)| (Some(error), truncated, original_bytes))
            .unwrap_or((None, false, 0));
        result.error = error;

        if output_truncated || error_truncated {
            let mut metadata = result.metadata;
            if output_truncated {
                metadata = metadata_with(metadata, "output_truncated", json!(true));
                metadata = metadata_with(
                    metadata,
                    "output_original_bytes",
                    json!(output_original_bytes),
                );
            }
            if error_truncated {
                metadata = metadata_with(metadata, "error_truncated", json!(true));
                metadata = metadata_with(
                    metadata,
                    "error_original_bytes",
                    json!(error_original_bytes),
                );
            }
            metadata = metadata_with(metadata, "max_output_bytes", json!(self.max_output_bytes));
            result.metadata = metadata;
        }

        result
    }

    fn merge_granted_permissions(&self, result: &ToolResult) {
        if !result.ok {
            return;
        }
        let Some(permissions) = result.metadata.get("granted_permissions") else {
            return;
        };
        let Some(permissions) = permissions.as_array() else {
            return;
        };
        self.permission_grants.grant(
            permissions
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
}

#[async_trait]
pub(crate) trait ToolExecutionObserver: Send + Sync {
    async fn tool_call_requested(&self, call: &ToolCall) -> Result<()>;
    async fn approval_requested(&self, call: &ToolCall, reason: &str) -> Result<()>;
    async fn approval_resolved(&self, call: &ToolCall, approved: bool) -> Result<()>;
    async fn tool_finished(&self, result: &ToolResult) -> Result<()>;
}

struct NoopToolExecutionObserver;

#[async_trait]
impl ToolExecutionObserver for NoopToolExecutionObserver {
    async fn tool_call_requested(&self, _call: &ToolCall) -> Result<()> {
        Ok(())
    }

    async fn approval_requested(&self, _call: &ToolCall, _reason: &str) -> Result<()> {
        Ok(())
    }

    async fn approval_resolved(&self, _call: &ToolCall, _approved: bool) -> Result<()> {
        Ok(())
    }

    async fn tool_finished(&self, _result: &ToolResult) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "bound_tools_tests.rs"]
mod tests;
