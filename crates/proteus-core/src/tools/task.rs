//! Facade-tool for the replaceable `SubagentRunner` slot.
//!
//! The tool owns only request shaping and optional worktree lifecycle. The
//! child agent loop remains behind `SubagentToolHost`/`SubagentRunner`, while
//! visibility, timeout, events and output bounds stay in the normal
//! `ToolRegistry -> ToolOrchestrator -> Tool::invoke` path.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    contracts::{
        SubagentRequest, SubagentResult, SubagentRoleSpec, SubagentStatus, Tool, ToolContext,
        ToolRegistry, ToolSource,
    },
    domain::{ToolCall, ToolResult, ToolSpec},
};

mod spec;
mod workspace_lifecycle;

#[cfg(test)]
mod tests;

use spec::{is_parallel_eligible, task_tool_spec};
use workspace_lifecycle::{
    child_task_id, error_result_with_workspace, finalize_workspace, prepare_workspace,
};

pub const TASK_TOOL: &str = "task";

pub fn register_task_tool(
    tools: &mut ToolRegistry,
    roles: Vec<SubagentRoleSpec>,
    timeout_ms: u64,
) -> Result<()> {
    if roles.is_empty() {
        return Ok(());
    }
    tools.register_with_source(
        ToolSource::builtin("subagent"),
        TaskTool::new(roles, timeout_ms),
    )
}

#[derive(Clone)]
pub struct TaskTool {
    roles: Vec<SubagentRoleSpec>,
    timeout_ms: u64,
}

impl TaskTool {
    pub fn new(roles: Vec<SubagentRoleSpec>, timeout_ms: u64) -> Self {
        Self { roles, timeout_ms }
    }

    fn role(&self, call: &ToolCall) -> Option<&SubagentRoleSpec> {
        let role = call.args.get("agent_type").and_then(Value::as_str)?;
        self.roles.iter().find(|candidate| candidate.name == role)
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn spec(&self) -> ToolSpec {
        task_tool_spec(&self.roles, self.timeout_ms)
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let parent_task = ctx
            .task
            .clone()
            .ok_or_else(|| anyhow!("task tool requires current AgentTask in ToolContext"))?;
        let subagent = ctx
            .subagent
            .clone()
            .ok_or_else(|| anyhow!("task tool requires SubagentToolHost capability"))?;
        let role = self.role(call).cloned();
        let mut request = match parse_task_call(call, parent_task) {
            Ok(request) => request,
            Err(message) => return Ok(task_error(call, message)),
        };
        let Some(role) = role else {
            return Ok(task_error(
                call,
                format!("unknown subagent role '{}'", request.role),
            ));
        };

        let workspace =
            match prepare_workspace(&mut request, call, &role, subagent.session_id()).await {
                Ok(workspace) => workspace,
                Err(message) => return Ok(task_error(call, message)),
            };
        match subagent.run_subagent(request).await {
            Ok(result) => {
                let note = finalize_workspace(workspace, &result).await;
                Ok(result_to_tool_result(call, result, note))
            }
            Err(error) => {
                Ok(error_result_with_workspace(call, format!("{error:#}"), workspace).await)
            }
        }
    }
}

fn parse_task_call(
    call: &ToolCall,
    parent_task: crate::domain::AgentTask,
) -> Result<SubagentRequest, String> {
    let args = call
        .args
        .as_object()
        .ok_or_else(|| "task args must be an object".to_owned())?;
    let agent_type = args
        .get("agent_type")
        .and_then(Value::as_str)
        .ok_or_else(|| "task requires string arg 'agent_type'".to_owned())?;
    let prompt = args
        .get("prompt")
        .and_then(Value::as_str)
        .ok_or_else(|| "task requires string arg 'prompt'".to_owned())?;
    let description = optional_string(args.get("description"), "description")?;
    let task_id = optional_string(args.get("task_id"), "task_id")?;

    let mut request = SubagentRequest::new(agent_type, prompt, parent_task);
    if let Some(description) = description {
        request = request.with_description(description);
    }
    if let Some(task_id) = task_id {
        request = request.with_metadata(json!({ "task_id": task_id }));
    }
    Ok(request)
}

fn optional_string(value: Option<&Value>, name: &str) -> Result<Option<String>, String> {
    match value {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("task arg '{name}' must be a string when provided")),
        None => Ok(None),
    }
}

pub(crate) fn calls_are_parallel_eligible(calls: &[ToolCall], roles: &[SubagentRoleSpec]) -> bool {
    calls.len() >= 2
        && calls.iter().all(|call| {
            let role = call.args.get("agent_type").and_then(Value::as_str);
            role.and_then(|name| roles.iter().find(|role| role.name == name))
                .is_some_and(is_parallel_eligible)
        })
}

fn task_error(call: &ToolCall, message: String) -> ToolResult {
    ToolResult::error(call.id.clone(), message).with_metadata(json!({ "tool": TASK_TOOL }))
}

fn result_to_tool_result(
    call: &ToolCall,
    result: SubagentResult,
    workspace_note: Option<String>,
) -> ToolResult {
    let task_id = child_task_id(&result);
    let mut output = result.summary;
    if result.status == SubagentStatus::TokenBudgetExceeded {
        let spent = result
            .usage
            .as_ref()
            .map(|usage| usage.total_tokens())
            .unwrap_or(0);
        if task_id.is_some() {
            output.push_str(&format!(
                "\n\n[token budget exhausted ({spent} tokens spent); the summary above may be incomplete — resume with task_id to continue]"
            ));
        } else {
            output.push_str(&format!(
                "\n\n[token budget exhausted ({spent} tokens spent); the summary above may be incomplete — no resumable child state was retained]"
            ));
        }
    }
    if let Some(task_id) = task_id {
        output.push_str(&format!("\n\n[task_id: {task_id}]"));
    }
    if let Some(note) = workspace_note {
        output.push('\n');
        output.push_str(&note);
    }

    ToolResult::ok(call.id.clone(), output).with_metadata(json!({
        "tool": TASK_TOOL,
        "status": result.status,
        "iterations": result.iterations,
        "child_thread_id": result.child_thread_id,
        "usage": result.usage,
    }))
}
