use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::time::timeout;

use crate::{
    contracts::{
        ApprovalRequest, PolicyContext, PolicyVisibilityContext, RuntimeContext, ToolContext,
    },
    domain::{AgentTask, Event, PolicyDecision, ToolCall, ToolResult, ToolSpec},
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
                match ctx.policy.evaluate_visibility(&PolicyVisibilityContext {
                    cwd: cwd.to_path_buf(),
                    tool_spec: spec.clone(),
                }) {
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
        ctx.emit(Event::ToolCallRequested { call: call.clone() })
            .await?;

        let tool_spec = ctx.tools.spec(&call.name).ok();
        if let Some(spec) = tool_spec.as_ref()
            && let Some(error) = validate_tool_call_args(&call, spec)
        {
            let result = ToolResult {
                call_id: call.id.clone(),
                ok: false,
                output: String::new(),
                content: Vec::new(),
                error: Some(error),
                metadata: json!({
                    "tool": call.name,
                    "validation_error": true,
                }),
            };
            return self.finish(ctx, result).await;
        }

        let decision = self.evaluate_access(ctx, &task.cwd, &call, tool_spec.clone());

        match decision {
            PolicyDecision::Allow => self.invoke_allowed(ctx, task, &call, tool_spec).await,
            PolicyDecision::Ask { reason } => {
                ctx.emit(Event::ApprovalRequested {
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
                ctx.emit(Event::ApprovalResolved {
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
                    content: Vec::new(),
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
                    content: Vec::new(),
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

        ctx.policy.evaluate(
            call,
            &PolicyContext {
                cwd: cwd.to_path_buf(),
                tool_spec: Some(spec),
            },
        )
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
            tool.invoke(call, ToolContext::new(task.cwd.clone())),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => ToolResult {
                call_id: call.id.clone(),
                ok: false,
                output: String::new(),
                content: Vec::new(),
                error: Some(error.to_string()),
                metadata: json!({ "tool": call.name }),
            },
            Err(_) => ToolResult {
                call_id: call.id.clone(),
                ok: false,
                output: String::new(),
                content: Vec::new(),
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
        ctx.emit(Event::ToolFinished {
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

fn validate_tool_call_args(call: &ToolCall, spec: &ToolSpec) -> Option<String> {
    let schema = spec.input_schema.as_object()?;
    let required_args = schema.get("required").and_then(Value::as_array);
    let properties = schema.get("properties").and_then(Value::as_object);
    let expects_object = required_args.is_some() || properties.is_some();

    if expects_object && !call.args.is_object() {
        return Some(format!("tool '{}' requires object args", call.name));
    }

    let args = call.args.as_object()?;
    for required in required_args.into_iter().flatten() {
        let Some(name) = required.as_str() else {
            continue;
        };
        let property = properties.and_then(|properties| properties.get(name));
        let expected_types = property.map(schema_type_names).unwrap_or_default();
        let Some(value) = args.get(name) else {
            return Some(required_arg_error(&call.name, name, &expected_types));
        };
        if !expected_types.is_empty()
            && !expected_types
                .iter()
                .any(|expected_type| value_matches_schema_type(value, expected_type))
        {
            return Some(required_arg_error(&call.name, name, &expected_types));
        }
    }

    None
}

fn schema_type_names(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(type_name)) => vec![type_name.as_str()],
        Some(Value::Array(type_names)) => type_names.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn value_matches_schema_type(value: &Value, expected_type: &str) -> bool {
    match expected_type {
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "object" => value.is_object(),
        "string" => value.is_string(),
        _ => true,
    }
}

fn required_arg_error(tool_name: &str, arg_name: &str, expected_types: &[&str]) -> String {
    let Some(expected_type) = expected_types.first() else {
        return format!("tool '{tool_name}' requires arg '{arg_name}'");
    };
    format!("tool '{tool_name}' requires {expected_type} arg '{arg_name}'")
}
