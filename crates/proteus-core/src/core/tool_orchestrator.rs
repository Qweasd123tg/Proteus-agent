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

const DEFAULT_MAX_OUTPUT_BYTES: usize = 200_000;

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
        // Codex-модели часто вызывают apply_patch как shell-команду
        // (`apply_patch <<'EOF' ...`). Роутим такой вызов в настоящий
        // apply_patch tool, чтобы патч прошёл patch-flow (policy, applier,
        // события) вместо падения в шелле на несуществующем бинаре.
        let call = intercept_apply_patch_call(ctx, &call).unwrap_or(call);
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
                    let result = self.invoke_allowed(ctx, task, &call, tool_spec).await?;
                    // Approval-gated grants: только результат явно одобренного
                    // вызова может выдать turn-scoped права (см. contracts
                    // TurnPermissionGrants).
                    merge_granted_permissions(ctx, &result);
                    return Ok(result);
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

        ctx.policy.evaluate(
            call,
            &PolicyContext::new(cwd.to_path_buf(), Some(spec))
                .with_granted_permissions(ctx.turn_grants.snapshot()),
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
        let tool_ctx = ToolContext {
            cwd: task.cwd.clone(),
            cancellation: ctx.cancellation.clone(),
            user_input: Some(ctx.user_input.clone()),
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
}

fn truncate_utf8(value: String, max_bytes: usize, kind: &str) -> (String, bool, usize) {
    let original_bytes = value.len();
    if original_bytes <= max_bytes {
        return (value, false, original_bytes);
    }

    let mut prefix_limit = max_bytes;
    loop {
        let prefix = utf8_prefix(&value, prefix_limit);
        let notice = truncation_notice(kind, prefix.len(), original_bytes);
        let combined_len = prefix.len() + notice.len();
        if combined_len <= max_bytes {
            return (format!("{prefix}{notice}"), true, original_bytes);
        }

        if prefix_limit == 0 {
            return (
                utf8_prefix(&notice, max_bytes).to_owned(),
                true,
                original_bytes,
            );
        }

        let overflow = combined_len - max_bytes;
        prefix_limit = prefix_limit.saturating_sub(overflow.max(1));
    }
}

fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }

    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn truncation_notice(kind: &str, shown_bytes: usize, original_bytes: usize) -> String {
    format!(
        "\n\n[tool {kind} truncated: showing first {shown_bytes} of {original_bytes} bytes. \
Re-run the tool with a narrower range or explicit limit for the remaining content.]"
    )
}

/// Перехват shell-стиля apply_patch: если команда — это вызов `apply_patch`
/// с патчем (heredoc, кавычки или голый аргумент) и tool `apply_patch`
/// зарегистрирован, переписывает вызов на него с тем же `call_id`.
fn intercept_apply_patch_call(ctx: &RuntimeContext, call: &ToolCall) -> Option<ToolCall> {
    if call.name != "shell" && call.name != "exec_command" {
        return None;
    }
    let command = call
        .args
        .get("command")
        .or_else(|| call.args.get("cmd"))
        .and_then(Value::as_str)?;
    let patch = extract_apply_patch_body(command)?;
    ctx.tools.spec("apply_patch").ok()?;
    Some(ToolCall::new(
        call.id.clone(),
        "apply_patch".to_owned(),
        json!({ "patch": patch }),
    ))
}

/// Достаёт тело патча из shell-вызова `apply_patch`. Поддерживает heredoc
/// (`apply_patch <<'EOF' ... EOF`), одинарные/двойные кавычки и голый
/// аргумент; тело обязано начинаться с `*** Begin Patch`.
fn extract_apply_patch_body(command: &str) -> Option<String> {
    let rest = command.trim().strip_prefix("apply_patch")?.trim();
    if let Some(heredoc) = rest.strip_prefix("<<") {
        let (delimiter_line, body) = heredoc.split_once('\n')?;
        let delimiter = delimiter_line
            .trim()
            .trim_start_matches('-')
            .trim_matches(|quote| quote == '\'' || quote == '"');
        if delimiter.is_empty() {
            return None;
        }
        let body = body.trim_end().strip_suffix(delimiter)?;
        return normalized_patch(body.strip_suffix('\n').unwrap_or(body));
    }
    for quote in ['\'', '"'] {
        if let Some(inner) = rest
            .strip_prefix(quote)
            .and_then(|inner| inner.strip_suffix(quote))
        {
            return normalized_patch(inner);
        }
    }
    normalized_patch(rest)
}

fn normalized_patch(text: &str) -> Option<String> {
    let text = text.trim();
    text.starts_with("*** Begin Patch").then(|| text.to_owned())
}

/// Мержит `metadata.granted_permissions` успешного approved-результата в
/// гранты текущего хода. Вызывается только с approved-пути `execute`.
fn merge_granted_permissions(ctx: &RuntimeContext, result: &ToolResult) {
    if !result.ok {
        return;
    }
    let Some(permissions) = result.metadata.get("granted_permissions") else {
        return;
    };
    let Some(permissions) = permissions.as_array() else {
        return;
    };
    ctx.turn_grants.grant(
        permissions
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned),
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_apply_patch_body_supports_heredoc_quotes_and_bare() {
        let patch = "*** Begin Patch\n*** Add File: hi.txt\n+hi\n*** End Patch";

        let heredoc = format!("apply_patch <<'EOF'\n{patch}\nEOF");
        assert_eq!(extract_apply_patch_body(&heredoc).as_deref(), Some(patch));

        let heredoc_plain = format!("apply_patch <<EOF\n{patch}\nEOF\n");
        assert_eq!(
            extract_apply_patch_body(&heredoc_plain).as_deref(),
            Some(patch)
        );

        let quoted = format!("apply_patch '{patch}'");
        assert_eq!(extract_apply_patch_body(&quoted).as_deref(), Some(patch));

        let bare = format!("apply_patch {patch}");
        assert_eq!(extract_apply_patch_body(&bare).as_deref(), Some(patch));
    }

    #[test]
    fn extract_apply_patch_body_rejects_non_patch_commands() {
        assert_eq!(extract_apply_patch_body("cargo test"), None);
        assert_eq!(extract_apply_patch_body("apply_patch --help"), None);
        assert_eq!(
            extract_apply_patch_body("apply_patch <<'EOF'\nnot a patch\nEOF"),
            None
        );
        // git apply и прочие команды с подстрокой не матчатся.
        assert_eq!(
            extract_apply_patch_body("echo apply_patch <<'EOF'\n*** Begin Patch\nEOF"),
            None
        );
    }

    #[test]
    fn truncate_utf8_adds_visible_notice_within_limit() {
        let original = "a".repeat(120);

        let (output, truncated, original_bytes) = truncate_utf8(original, 80, "output");

        assert!(truncated);
        assert_eq!(original_bytes, 120);
        assert!(output.len() <= 80);
        assert!(output.contains("[tool output truncated:"));
        assert!(output.contains("of 120 bytes"));
    }

    #[test]
    fn truncate_utf8_preserves_character_boundaries() {
        let original = "й".repeat(80);

        let (output, truncated, original_bytes) = truncate_utf8(original, 96, "error");

        assert!(truncated);
        assert_eq!(original_bytes, 160);
        assert!(output.len() <= 96);
        assert!(output.is_char_boundary(output.len()));
        assert!(output.contains("[tool error truncated:"));
    }
}
