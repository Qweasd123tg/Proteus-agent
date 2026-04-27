use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::time::timeout;

use crate::{
    contracts::{ApprovalRequest, PolicyContext, RuntimeContext, ToolContext},
    domain::{
        AgentTask, Event, PermissionMode, PolicyDecision, ToolCall, ToolResult, ToolSafety,
        ToolSpec,
    },
};

#[derive(Debug, Clone)]
pub struct ToolOrchestrator {
    default_timeout_ms: u64,
    max_output_bytes: usize,
}

impl Default for ToolOrchestrator {
    fn default() -> Self {
        Self {
            default_timeout_ms: 30_000,
            max_output_bytes: 20_000,
        }
    }
}

impl ToolOrchestrator {
    pub fn new(default_timeout_ms: u64, max_output_bytes: usize) -> Self {
        Self {
            default_timeout_ms,
            max_output_bytes,
        }
    }

    pub fn visible_tool_specs(&self, ctx: &RuntimeContext, cwd: &Path) -> Vec<ToolSpec> {
        ctx.tools
            .specs()
            .into_iter()
            .filter(|spec| {
                let call = ToolCall {
                    id: crate::domain::new_call_id(),
                    name: spec.name.clone(),
                    args: serde_json::Value::Null,
                };
                match self.evaluate_access(ctx, cwd, &call, Some(spec.clone())) {
                    PolicyDecision::Allow => true,
                    PolicyDecision::Ask { .. } => ctx.approval.can_request_approval(),
                    PolicyDecision::Deny { .. } => false,
                }
            })
            .collect()
    }

    pub async fn execute(
        &self,
        ctx: &RuntimeContext,
        task: &AgentTask,
        call: ToolCall,
    ) -> Result<ToolResult> {
        ctx.event_sink
            .append(Event::ToolCallRequested { call: call.clone() })
            .await?;

        let tool_spec = ctx.tools.spec(&call.name).ok();
        let decision = self.evaluate_access(ctx, &task.cwd, &call, tool_spec.clone());

        match decision {
            PolicyDecision::Allow => self.invoke_allowed(ctx, task, &call, tool_spec).await,
            PolicyDecision::Ask { reason } => {
                ctx.event_sink
                    .append(Event::ApprovalRequested {
                        call_id: call.id.clone(),
                        reason: reason.clone(),
                    })
                    .await?;
                let approval = ctx
                    .approval
                    .request_approval(ApprovalRequest {
                        call: call.clone(),
                        cwd: task.cwd.clone(),
                        reason: reason.clone(),
                        tool_spec: tool_spec.clone(),
                    })
                    .await?;
                ctx.event_sink
                    .append(Event::ApprovalResolved {
                        call_id: call.id.clone(),
                        approved: approval.approved,
                    })
                    .await?;
                if approval.approved {
                    return self.invoke_allowed(ctx, task, &call, tool_spec).await;
                }

                let result = ToolResult {
                    call_id: call.id.clone(),
                    ok: false,
                    output: String::new(),
                    error: Some(
                        approval
                            .note
                            .unwrap_or_else(|| format!("tool call was not approved: {reason}")),
                    ),
                    metadata: serde_json::Value::Null,
                };
                self.finish(ctx, result).await
            }
            PolicyDecision::Deny { reason } => {
                let result = ToolResult {
                    call_id: call.id.clone(),
                    ok: false,
                    output: String::new(),
                    error: Some(reason),
                    metadata: serde_json::Value::Null,
                };
                self.finish(ctx, result).await
            }
        }
    }

    fn evaluate_access(
        &self,
        ctx: &RuntimeContext,
        cwd: &Path,
        call: &ToolCall,
        tool_spec: Option<ToolSpec>,
    ) -> PolicyDecision {
        let Some(spec) = tool_spec else {
            return PolicyDecision::Deny {
                reason: format!("unknown tool: {}", call.name),
            };
        };

        match ctx.permission_mode {
            PermissionMode::Plan => match spec.safety {
                ToolSafety::ReadOnly => PolicyDecision::Allow,
                _ => PolicyDecision::Deny {
                    reason: format!(
                        "permission mode plan allows only read-only tools: {}",
                        call.name
                    ),
                },
            },
            PermissionMode::Auto => match spec.safety {
                ToolSafety::ReadOnly | ToolSafety::WritesFiles => PolicyDecision::Allow,
                ToolSafety::RunsCommands | ToolSafety::Network | ToolSafety::Dangerous => {
                    PolicyDecision::Deny {
                        reason: format!(
                            "permission mode auto denies command, network, and dangerous tools: {}",
                            call.name
                        ),
                    }
                }
            },
            PermissionMode::Normal => ctx.policy.evaluate(
                call,
                &PolicyContext {
                    cwd: cwd.to_path_buf(),
                    tool_spec: Some(spec),
                },
            ),
        }
    }

    async fn invoke_allowed(
        &self,
        ctx: &RuntimeContext,
        task: &AgentTask,
        call: &ToolCall,
        tool_spec: Option<ToolSpec>,
    ) -> Result<ToolResult> {
        let tool = ctx
            .tools
            .get(&call.name)
            .ok_or_else(|| anyhow!("unknown tool: {}", call.name))?;
        let timeout_ms = tool_spec
            .as_ref()
            .and_then(|spec| spec.timeout_ms)
            .unwrap_or(self.default_timeout_ms);
        let started = Instant::now();
        let result = match timeout(
            Duration::from_millis(timeout_ms),
            tool.invoke(
                call,
                ToolContext {
                    cwd: task.cwd.clone(),
                },
            ),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => ToolResult {
                call_id: call.id.clone(),
                ok: false,
                output: String::new(),
                error: Some(error.to_string()),
                metadata: json!({ "tool": call.name }),
            },
            Err(_) => ToolResult {
                call_id: call.id.clone(),
                ok: false,
                output: String::new(),
                error: Some(format!("tool timed out after {timeout_ms}ms")),
                metadata: json!({
                    "tool": call.name,
                    "timed_out": true,
                    "timeout_ms": timeout_ms,
                }),
            },
        };

        let mut result = self.truncate_result(result);
        result.metadata = metadata_with(
            result.metadata,
            "duration_ms",
            json!(started.elapsed().as_millis() as u64),
        );
        self.finish(ctx, result).await
    }

    async fn finish(&self, ctx: &RuntimeContext, result: ToolResult) -> Result<ToolResult> {
        ctx.event_sink
            .append(Event::ToolFinished {
                result: result.clone(),
            })
            .await?;
        Ok(result)
    }

    fn truncate_result(&self, mut result: ToolResult) -> ToolResult {
        let (output, output_truncated, output_original_bytes) =
            truncate_utf8(result.output, self.max_output_bytes);
        result.output = output;

        let (error, error_truncated, error_original_bytes) = result
            .error
            .map(|error| truncate_utf8(error, self.max_output_bytes))
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
}

fn truncate_utf8(value: String, max_bytes: usize) -> (String, bool, usize) {
    let original_bytes = value.len();
    if original_bytes <= max_bytes {
        return (value, false, original_bytes);
    }

    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_owned(), true, original_bytes)
}

fn metadata_with(metadata: Value, key: &str, value: Value) -> Value {
    let mut object = match metadata {
        Value::Object(object) => object,
        _ => serde_json::Map::new(),
    };
    object.insert(key.to_owned(), value);
    Value::Object(object)
}
