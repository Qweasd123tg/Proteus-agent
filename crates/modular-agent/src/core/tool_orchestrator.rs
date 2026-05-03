use std::{
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
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
                match ctx
                    .policy
                    .evaluate_visibility(&PolicyVisibilityContext::new(
                        cwd.to_path_buf(),
                        spec.clone(),
                    )) {
                    PolicyDecision::Allow => true,
                    PolicyDecision::Ask { .. } => ctx.approval.can_request_approval(),
                    PolicyDecision::Deny { .. } => false,
                    _ => false,
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
        if ctx.is_cancelled() {
            anyhow::bail!("turn canceled by client");
        }
        ctx.emit(Event::ToolCallRequested { call: call.clone() })
            .await?;

        let tool_spec = ctx.tools.spec(&call.name).ok();
        if let Some(spec) = tool_spec.as_ref()
            && let Some(error) = validate_tool_call_args(&call, spec)
        {
            let result = ToolResult::error(call.id.clone(), error).with_metadata(json!({
                "tool": call.name,
                "validation_error": true,
            }));
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
                let approval_request = ctx.approval.request_approval(ApprovalRequest::new(
                    call.clone(),
                    task.cwd.clone(),
                    reason.clone(),
                    tool_spec.clone(),
                ));
                let approval = tokio::select! {
                    result = approval_request => result?,
                    _ = ctx.cancellation.cancelled() => {
                        return Err(anyhow!("turn canceled by client"));
                    }
                };
                ctx.emit(Event::ApprovalResolved {
                    call_id: call.id.clone(),
                    approved: approval.approved,
                })
                .await?;
                if approval.approved {
                    return self.invoke_allowed(ctx, task, &call, tool_spec).await;
                }

                let result = ToolResult::error(
                    call.id.clone(),
                    approval
                        .note
                        .unwrap_or_else(|| format!("tool call was not approved: {reason}")),
                );
                self.finish(ctx, result).await
            }
            PolicyDecision::Deny { reason } => {
                let result = ToolResult::error(call.id.clone(), reason);
                self.finish(ctx, result).await
            }
            other => {
                let result = ToolResult::error(
                    call.id.clone(),
                    format!("unsupported policy decision: {other:?}"),
                );
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

        ctx.policy
            .evaluate(call, &PolicyContext::new(cwd.to_path_buf(), Some(spec)))
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
        let tool_ctx = ToolContext {
            cwd: task.cwd.clone(),
            cancellation: ctx.cancellation.clone(),
        };
        let result = tokio::select! {
            result = timeout(Duration::from_millis(timeout_ms), tool.invoke(call, tool_ctx)) => {
                match result {
                    Ok(Ok(result)) => result,
                    Ok(Err(error)) => ToolResult::error(call.id.clone(), error.to_string())
                        .with_metadata(json!({ "tool": call.name })),
                    Err(_) => ToolResult::error(
                        call.id.clone(),
                        format!("tool timed out after {timeout_ms}ms"),
                    )
                    .with_metadata(json!({
                        "tool": call.name,
                        "timed_out": true,
                        "timeout_ms": timeout_ms,
                    })),
                }
            }
            _ = ctx.cancellation.cancelled() => {
                ToolResult::error(call.id.clone(), "tool call canceled")
                    .with_metadata(json!({
                        "tool": call.name,
                        "canceled": true,
                    }))
            }
        };

        let mut result = self.truncate_result(ctx, task, &call.name, result).await;
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

    async fn truncate_result(
        &self,
        ctx: &RuntimeContext,
        task: &AgentTask,
        tool_name: &str,
        mut result: ToolResult,
    ) -> ToolResult {
        let (output, output_truncated, output_original_bytes) =
            truncate_utf8(&result.output, self.max_output_bytes);
        let output_artifact = if output_truncated {
            self.write_tool_artifact(
                ctx,
                task,
                tool_name,
                &result.call_id,
                "output",
                &result.output,
            )
            .await
        } else {
            None
        };
        result.output = output;

        let (error, error_truncated, error_original_bytes, error_artifact) = match result
            .error
            .take()
        {
            Some(error) => {
                let (truncated_error, truncated, original_bytes) =
                    truncate_utf8(&error, self.max_output_bytes);
                let artifact = if truncated {
                    self.write_tool_artifact(ctx, task, tool_name, &result.call_id, "error", &error)
                        .await
                } else {
                    None
                };
                (Some(truncated_error), truncated, original_bytes, artifact)
            }
            None => (None, false, 0, None),
        };
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
                if let Some(artifact) = output_artifact {
                    result.output.push_str(&format!(
                        "\n\n[output truncated to {} bytes; full output saved to {}]",
                        self.max_output_bytes,
                        artifact.relative_path.display()
                    ));
                    metadata = metadata_with(
                        metadata,
                        "output_artifact_path",
                        json!(artifact.relative_path.to_string_lossy()),
                    );
                    metadata =
                        metadata_with(metadata, "output_artifact_bytes", json!(artifact.bytes));
                }
            }
            if error_truncated {
                metadata = metadata_with(metadata, "error_truncated", json!(true));
                metadata = metadata_with(
                    metadata,
                    "error_original_bytes",
                    json!(error_original_bytes),
                );
                if let Some(artifact) = error_artifact {
                    let note = format!(
                        "\n\n[error truncated to {} bytes; full error saved to {}]",
                        self.max_output_bytes,
                        artifact.relative_path.display()
                    );
                    result.error = Some(match result.error.take() {
                        Some(mut error) => {
                            error.push_str(&note);
                            error
                        }
                        None => note,
                    });
                    metadata = metadata_with(
                        metadata,
                        "error_artifact_path",
                        json!(artifact.relative_path.to_string_lossy()),
                    );
                    metadata =
                        metadata_with(metadata, "error_artifact_bytes", json!(artifact.bytes));
                }
            }
            metadata = metadata_with(metadata, "max_output_bytes", json!(self.max_output_bytes));
            result.metadata = metadata;
        }

        result
    }

    async fn write_tool_artifact(
        &self,
        _ctx: &RuntimeContext,
        task: &AgentTask,
        tool_name: &str,
        call_id: &str,
        stream: &str,
        content: &str,
    ) -> Option<ToolArtifactRef> {
        let relative_path = tool_artifact_relative_path(tool_name, call_id, stream);
        match write_workspace_text_artifact(&task.cwd, &relative_path, content).await {
            Ok(()) => Some(ToolArtifactRef {
                relative_path,
                bytes: content.len(),
            }),
            Err(error) => {
                eprintln!(
                    "warning: failed to write tool output artifact {}: {error:#}",
                    task.cwd.join(&relative_path).display()
                );
                None
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ToolArtifactRef {
    relative_path: PathBuf,
    bytes: usize,
}

async fn write_workspace_text_artifact(
    workspace: &Path,
    relative_path: &Path,
    content: &str,
) -> Result<()> {
    ensure_relative_artifact_path(relative_path)?;
    let path = workspace.join(relative_path);
    let parent = relative_path
        .parent()
        .ok_or_else(|| anyhow!("artifact path has no parent: {}", relative_path.display()))?;
    reject_existing_symlink_components(workspace, parent).await?;
    tokio::fs::create_dir_all(workspace.join(parent))
        .await
        .with_context(|| format!("failed to create {}", workspace.join(parent).display()))?;
    tokio::fs::write(path, content)
        .await
        .with_context(|| format!("failed to write {}", workspace.join(relative_path).display()))?;
    Ok(())
}

fn ensure_relative_artifact_path(path: &Path) -> Result<()> {
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("artifact path must stay inside workspace: {}", path.display());
            }
        }
    }
    Ok(())
}

async fn reject_existing_symlink_components(workspace: &Path, relative_dir: &Path) -> Result<()> {
    let mut current = workspace.to_path_buf();
    for component in relative_dir.components() {
        match component {
            Component::Normal(part) => {
                current.push(part);
                match tokio::fs::symlink_metadata(&current).await {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        anyhow::bail!(
                            "artifact directory must not contain symlink component: {}",
                            current.display()
                        );
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Err(error) => return Err(error).with_context(|| {
                        format!("failed to inspect artifact path {}", current.display())
                    }),
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "artifact directory must stay inside workspace: {}",
                    relative_dir.display()
                );
            }
        }
    }
    Ok(())
}

fn tool_artifact_relative_path(tool_name: &str, call_id: &str, stream: &str) -> PathBuf {
    PathBuf::from(".agent")
        .join("tool-outputs")
        .join(sanitize_path_segment(tool_name))
        .join(format!(
            "{}-{}.txt",
            sanitize_path_segment(call_id),
            sanitize_path_segment(stream)
        ))
}

fn sanitize_path_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "unknown".to_owned()
    } else {
        sanitized
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> (String, bool, usize) {
    let original_bytes = value.len();
    if original_bytes <= max_bytes {
        return (value.to_owned(), false, original_bytes);
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
