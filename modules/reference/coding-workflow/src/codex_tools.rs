//! Codex-owned adaptation from a model call to an explicit host operation.
//! Model history stays unchanged; checkpoint and execution use the same calls.
use proteus_contracts::{
    domain::{ToolCall, ToolCallSurface, ToolResult, ToolSafety, ToolSpec},
    process_module::{ProcessModuleError, WorkflowModuleHostMut, WorkflowModuleInput},
};
use serde_json::{Value, json};

use crate::host::{execute_or_handle_tool, execute_tools};
use crate::scaffold::TurnScaffold;

/// One execution path for calls from successful and interrupted responses.
#[derive(Default)]
pub(crate) struct CodexToolRun {
    pub(crate) tool_rounds: usize,
    pub(crate) executed_tools: Vec<String>,
}

impl CodexToolRun {
    pub(crate) fn prepare(
        &mut self,
        host: &WorkflowModuleHostMut<'_>,
        turn: &mut TurnScaffold,
        calls: &[ToolCall],
        request_tools: &[ToolSpec],
    ) -> Result<CodexToolBatch, ProcessModuleError> {
        let batch = CodexToolBatch::prepare(calls, request_tools);
        turn.checkpoint(host, &batch.execution_calls())?;
        self.executed_tools
            .extend(calls.iter().map(|call| call.name.clone()));
        Ok(batch)
    }
}

pub(crate) struct CodexToolBatch {
    calls: Vec<Result<ToolCall, ToolResult>>,
}

impl CodexToolBatch {
    pub(crate) fn prepare(calls: &[ToolCall], request_tools: &[ToolSpec]) -> Self {
        Self {
            calls: calls
                .iter()
                .map(|call| {
                    // Visibility belongs to the original model request. A hidden
                    // shell must not become executable through a visible patch tool.
                    if !request_tools.iter().any(|spec| spec.name == call.name) {
                        let kind = if call.surface == ToolCallSurface::Freeform {
                            "custom tool call"
                        } else {
                            "call"
                        };
                        return Err(ToolResult::error(
                            call.id.clone(),
                            format!("unsupported {kind}: {}", call.name),
                        ));
                    }
                    Ok(intercept_apply_patch_call(call).unwrap_or_else(|| call.clone()))
                })
                .collect(),
        }
    }

    pub(crate) fn permits_parallel(&self, tools: &[ToolSpec]) -> bool {
        self.calls.iter().all(|call| match call {
            Ok(call) => tools
                .iter()
                .any(|tool| tool.name == call.name && tool.safety == ToolSafety::ReadOnly),
            Err(_) => false,
        })
    }

    pub(crate) fn execution_calls(&self) -> Vec<ToolCall> {
        self.calls
            .iter()
            .filter_map(|call| call.as_ref().ok().cloned())
            .collect()
    }

    pub(crate) fn execute(
        &self,
        host: &WorkflowModuleHostMut<'_>,
        input: &WorkflowModuleInput,
        phase: &str,
    ) -> Result<Vec<ToolResult>, ProcessModuleError> {
        if self.calls.iter().all(Result::is_ok) {
            return execute_tools(host, input, &self.execution_calls(), phase);
        }
        self.calls
            .iter()
            .map(|call| match call {
                Ok(call) => execute_or_handle_tool(host, input, call, phase),
                Err(result) => Ok(result.clone()),
            })
            .collect()
    }
}

fn intercept_apply_patch_call(call: &ToolCall) -> Option<ToolCall> {
    if call.surface != ToolCallSurface::Function
        || (call.name != "shell" && call.name != "exec_command")
    {
        return None;
    }
    // Invalid raw JSON must reach normal tool validation, never be repaired
    // from a different args object by the patch adapter.
    let parsed;
    let args = if let Some(raw) = &call.raw_arguments {
        parsed = serde_json::from_str::<Value>(raw).ok()?;
        &parsed
    } else {
        &call.args
    };
    let command = args
        .get("command")
        .or_else(|| args.get("cmd"))
        .and_then(Value::as_str)?;
    let patch = extract_apply_patch_body(command)?;
    // The host decides whether this target exists and may execute. Never fall
    // back to OS shell execution after a patch target denial or failure.
    Some(ToolCall::new(
        call.id.clone(),
        "apply_patch",
        json!({"patch": patch}),
    ))
}

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
        assert_eq!(
            extract_apply_patch_body("echo apply_patch <<'EOF'\n*** Begin Patch\nEOF"),
            None
        );
    }
}
