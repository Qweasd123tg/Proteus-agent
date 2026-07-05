use proteus_contracts::{
    contracts::{SubagentRequest, SubagentResult, SubagentRoleSpec},
    domain::{ToolCall, ToolResult, ToolSafety, ToolSpec},
    plugin::{PluginWorkflowError, PluginWorkflowHostMut, PluginWorkflowInput},
};
use serde_json::{Value, json};

use crate::host::run_subagent;

pub const TASK_TOOL: &str = "task";

pub fn is_task_tool(name: &str) -> bool {
    name == TASK_TOOL
}

pub fn task_tool_spec(roles: &[SubagentRoleSpec]) -> Option<ToolSpec> {
    if roles.is_empty() {
        return None;
    }

    let role_description = roles
        .iter()
        .map(|role| format!("- {}: {}", role.name, role.description))
        .collect::<Vec<_>>()
        .join("\n");

    Some(
        ToolSpec::new(
            TASK_TOOL,
            "Delegate a focused task to an isolated Proteus subagent role and return its summary.\n\
The subagent starts with a FRESH context, so include all necessary background in the prompt; parent history is not passed.\n\
The subagent's work is NOT visible to the user: its summary comes back only to you, and you must relay important findings in your reply.\n\
Reuse task_id from a previous task result to continue that subagent with its accumulated context instead of starting fresh.\n\
Delegate when a subtask is self-contained and its full trace would pollute your context, such as research, verification, or broad searches.\n\
Choose the role that best matches the delegated work and keep the prompt specific.\n\
Do the work yourself when it needs your accumulated context, close supervision, or tight iteration with the user.\n\
Tasks run sequentially; start another task only after the current delegated work has returned.",
            json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "Short 3-5 word label for the delegated task."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Full prompt for the subagent. Include all necessary context because parent history is not passed."
                    },
                    "agent_type": {
                        "type": "string",
                        "description": format!("Subagent role to use. Available roles:\n{role_description}")
                    },
                    "task_id": {
                        "type": "string",
                        "description": "Resume a previous subagent task with its accumulated context instead of starting fresh. Use the task_id from a previous task result."
                    }
                },
                "required": ["prompt", "agent_type"]
            }),
            ToolSafety::WritesFiles,
        )
        .with_metadata(json!({
            "category": "proteus_subagent",
            "hot": true,
        })),
    )
}

pub fn append_task_tool(tools: &mut Vec<ToolSpec>, roles: &[SubagentRoleSpec]) {
    let Some(spec) = task_tool_spec(roles) else {
        return;
    };
    if !tools.iter().any(|tool| tool.name == TASK_TOOL) {
        tools.push(spec);
    }
}

pub fn handle_task_tool_call(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    call: &ToolCall,
) -> Result<ToolResult, PluginWorkflowError> {
    let Some(args) = call.args.as_object() else {
        return Ok(ToolResult::error(
            call.id.clone(),
            "task args must be an object",
        ));
    };
    let Some(agent_type) = args.get("agent_type").and_then(Value::as_str) else {
        return Ok(ToolResult::error(
            call.id.clone(),
            "task requires string arg 'agent_type'",
        ));
    };
    let Some(prompt) = args.get("prompt").and_then(Value::as_str) else {
        return Ok(ToolResult::error(
            call.id.clone(),
            "task requires string arg 'prompt'",
        ));
    };
    let description = match args.get("description") {
        Some(Value::String(description)) => Some(description.clone()),
        Some(_) => {
            return Ok(ToolResult::error(
                call.id.clone(),
                "task arg 'description' must be a string when provided",
            ));
        }
        None => None,
    };
    let task_id = match args.get("task_id") {
        Some(Value::String(task_id)) => Some(task_id.clone()),
        Some(_) => {
            return Ok(ToolResult::error(
                call.id.clone(),
                "task arg 'task_id' must be a string when provided",
            ));
        }
        None => None,
    };

    let mut request = SubagentRequest::new(agent_type, prompt, input.task.clone());
    if let Some(description) = description {
        request = request.with_description(description);
    }
    if let Some(task_id) = task_id {
        request = request.with_metadata(json!({ "task_id": task_id }));
    }

    let result = match run_subagent(host, &request) {
        Ok(result) => result,
        Err(error) => {
            return Ok(
                ToolResult::error(call.id.clone(), error.message.as_str()).with_metadata(json!({
                    "tool": TASK_TOOL,
                })),
            );
        }
    };
    Ok(result_to_tool_result(call, result))
}

fn result_to_tool_result(call: &ToolCall, result: SubagentResult) -> ToolResult {
    let mut output = result.summary;
    if let Some(task_id) = result.child_thread_id.as_ref().and_then(|child_thread_id| {
        serde_json::to_value(child_thread_id)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
    }) {
        output.push_str("\n\n[task_id: ");
        output.push_str(&task_id);
        output.push(']');
    }

    ToolResult::ok(call.id.clone(), output).with_metadata(json!({
        "status": result.status,
        "iterations": result.iterations,
        "child_thread_id": result.child_thread_id,
        "usage": result.usage,
    }))
}
