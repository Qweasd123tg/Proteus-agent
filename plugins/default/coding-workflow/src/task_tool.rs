use proteus_contracts::{
    contracts::{SubagentHandle, SubagentRequest, SubagentResult, SubagentRoleSpec},
    domain::{Event, ToolCall, ToolResult, ToolSafety, ToolSpec},
    plugin::{PluginWorkflowError, PluginWorkflowHostMut, PluginWorkflowInput},
};
use serde_json::{Value, json};

use crate::host::{emit_event, run_subagent, spawn_subagent, wait_subagent};

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
        .map(|role| {
            if role.parallel_safe {
                format!("- {} (parallel-safe): {}", role.name, role.description)
            } else {
                format!("- {}: {}", role.name, role.description)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let concurrency_line = if roles.iter().any(|role| role.parallel_safe) {
        "Several task calls issued in one reply run concurrently when every requested role is parallel-safe (marked in the role list); use that for independent read-only research. Any other combination runs sequentially, so start the next task only after the current one has returned."
    } else {
        "Tasks run sequentially; start another task only after the current delegated work has returned."
    };
    let description = format!(
        "Delegate a focused task to an isolated Proteus subagent role and return its summary.\n\
The subagent starts with a FRESH context, so include all necessary background in the prompt; parent history is not passed.\n\
The subagent's work is NOT visible to the user: its summary comes back only to you, and you must relay important findings in your reply.\n\
Reuse task_id from a previous task result to continue that subagent with its accumulated context instead of starting fresh.\n\
Delegate when a subtask is self-contained and its full trace would pollute your context, such as research, verification, or broad searches.\n\
Choose the role that best matches the delegated work and keep the prompt specific.\n\
Do the work yourself when it needs your accumulated context, close supervision, or tight iteration with the user.\n\
{concurrency_line}"
    );

    Some(
        ToolSpec::new(
            TASK_TOOL,
            description,
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

/// Батч task-вызовов можно исполнять конкурентно, только если каждый вызов
/// адресует существующую роль, объявленную `parallel_safe`.
pub fn all_roles_parallel_safe(calls: &[ToolCall], roles: &[SubagentRoleSpec]) -> bool {
    calls.iter().all(|call| {
        call.args
            .get("agent_type")
            .and_then(Value::as_str)
            .and_then(|name| roles.iter().find(|role| role.name == name))
            .is_some_and(|role| role.parallel_safe)
    })
}

/// Валидация аргументов task-вызова. Ошибка — текст для error ToolResult
/// (не инфраструктурный сбой workflow).
fn parse_task_call(
    input: &PluginWorkflowInput,
    call: &ToolCall,
) -> Result<SubagentRequest, String> {
    let Some(args) = call.args.as_object() else {
        return Err("task args must be an object".to_owned());
    };
    let Some(agent_type) = args.get("agent_type").and_then(Value::as_str) else {
        return Err("task requires string arg 'agent_type'".to_owned());
    };
    let Some(prompt) = args.get("prompt").and_then(Value::as_str) else {
        return Err("task requires string arg 'prompt'".to_owned());
    };
    let description = match args.get("description") {
        Some(Value::String(description)) => Some(description.clone()),
        Some(_) => {
            return Err("task arg 'description' must be a string when provided".to_owned());
        }
        None => None,
    };
    let task_id = match args.get("task_id") {
        Some(Value::String(task_id)) => Some(task_id.clone()),
        Some(_) => {
            return Err("task arg 'task_id' must be a string when provided".to_owned());
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
    Ok(request)
}

pub fn handle_task_tool_call(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    call: &ToolCall,
) -> Result<ToolResult, PluginWorkflowError> {
    let request = match parse_task_call(input, call) {
        Ok(request) => request,
        Err(message) => return Ok(ToolResult::error(call.id.clone(), message)),
    };

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

/// Конкурентное исполнение батча task-вызовов: сначала spawn всех детей
/// (ToolCallRequested в порядке вызовов), затем wait в том же порядке.
/// Ошибка одного вызова (аргументы, spawn, wait) даёт error ToolResult и
/// не прерывает остальных: уже запущенные дети дожидаются в любом случае.
pub fn handle_parallel_task_calls(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    calls: &[ToolCall],
) -> Result<Vec<ToolResult>, PluginWorkflowError> {
    enum Spawned {
        Running(SubagentHandle),
        Failed(String),
    }

    let mut spawned = Vec::with_capacity(calls.len());
    for call in calls {
        emit_event(host, &Event::ToolCallRequested { call: call.clone() })?;
        let outcome = match parse_task_call(input, call) {
            Ok(request) => match spawn_subagent(host, &request) {
                Ok(handle) => Spawned::Running(handle),
                Err(error) => Spawned::Failed(error.message.into_string()),
            },
            Err(message) => Spawned::Failed(message),
        };
        spawned.push(outcome);
    }

    let mut results = Vec::with_capacity(calls.len());
    for (call, outcome) in calls.iter().zip(spawned) {
        let result = match outcome {
            Spawned::Running(handle) => match wait_subagent(host, &handle) {
                Ok(result) => result_to_tool_result(call, result),
                Err(error) => ToolResult::error(call.id.clone(), error.message.as_str())
                    .with_metadata(json!({ "tool": TASK_TOOL })),
            },
            Spawned::Failed(message) => ToolResult::error(call.id.clone(), message)
                .with_metadata(json!({ "tool": TASK_TOOL })),
        };
        emit_event(
            host,
            &Event::ToolFinished {
                result: result.clone(),
            },
        )?;
        results.push(result);
    }
    Ok(results)
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
