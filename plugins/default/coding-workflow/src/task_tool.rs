use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use proteus_contracts::{
    contracts::{
        SubagentHandle, SubagentIsolation, SubagentRequest, SubagentResult, SubagentRoleSpec,
        SubagentWorkspaceRequest, WorkspaceInfo,
    },
    domain::{Event, ToolCall, ToolResult, ToolSafety, ToolSpec},
    plugin::{PluginWorkflowError, PluginWorkflowHostMut, PluginWorkflowInput},
};
use serde_json::{Value, json};

use crate::host::{
    cleanup_subagent_workspace, create_subagent_workspace, emit_event, run_subagent,
    spawn_subagent, subagent_roles, wait_subagent,
};

pub const TASK_TOOL: &str = "task";

pub fn is_task_tool(name: &str) -> bool {
    name == TASK_TOOL
}

/// Реестр живых worktree-ов по task_id ребёнка: resume того же task-а
/// должен попасть в ту же рабочую копию. In-memory, как и resumable-store
/// runner-ов: рестарт процесса теряет и снапшоты, и этот реестр.
fn workspaces() -> &'static Mutex<HashMap<String, WorkspaceInfo>> {
    static WORKSPACES: OnceLock<Mutex<HashMap<String, WorkspaceInfo>>> = OnceLock::new();
    WORKSPACES.get_or_init(Mutex::default)
}

fn lookup_workspace(task_id: &str) -> Option<WorkspaceInfo> {
    workspaces().lock().ok()?.get(task_id).cloned()
}

fn register_workspace(task_id: String, info: WorkspaceInfo) {
    if let Ok(mut map) = workspaces().lock() {
        map.insert(task_id, info);
    }
}

fn forget_workspace(task_id: &str) {
    if let Ok(mut map) = workspaces().lock() {
        map.remove(task_id);
    }
}

pub fn task_tool_spec(roles: &[SubagentRoleSpec]) -> Option<ToolSpec> {
    if roles.is_empty() {
        return None;
    }

    let role_description = roles
        .iter()
        .map(|role| {
            let mut markers = Vec::new();
            if role.parallel_safe {
                markers.push("parallel-safe");
            }
            if role.isolation == SubagentIsolation::Worktree {
                markers.push("worktree-isolated");
            }
            if markers.is_empty() {
                format!("- {}: {}", role.name, role.description)
            } else {
                format!(
                    "- {} ({}): {}",
                    role.name,
                    markers.join(", "),
                    role.description
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let concurrency_line = if roles.iter().any(is_parallel_eligible) {
        "Several task calls issued in one reply run concurrently when every requested role is parallel-safe or worktree-isolated (marked in the role list); use that for independent research or independent isolated changes. Any other combination runs sequentially, so start the next task only after the current one has returned."
    } else {
        "Tasks run sequentially; start another task only after the current delegated work has returned."
    };
    let worktree_line = if roles
        .iter()
        .any(|role| role.isolation == SubagentIsolation::Worktree)
    {
        "\nWorktree-isolated roles work in their own git worktree branched from the current HEAD, so they never touch your checkout. If the subagent changed anything, its result reports the worktree path and branch; NOTHING is merged automatically - review and merge that branch yourself (conflicts are normal work), then remove the worktree."
    } else {
        ""
    };
    let description = format!(
        "Delegate a focused task to an isolated Proteus subagent role and return its summary.\n\
The subagent starts with a FRESH context, so include all necessary background in the prompt; parent history is not passed.\n\
The subagent's work is NOT visible to the user: its summary comes back only to you, and you must relay important findings in your reply.\n\
Reuse task_id from a previous task result to continue that subagent with its accumulated context instead of starting fresh.\n\
Delegate when a subtask is self-contained and its full trace would pollute your context, such as research, verification, or broad searches.\n\
Choose the role that best matches the delegated work and keep the prompt specific.\n\
Do the work yourself when it needs your accumulated context, close supervision, or tight iteration with the user.\n\
{concurrency_line}{worktree_line}"
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

/// Роль пригодна для конкурентного батча: read-only (`parallel_safe`) либо
/// пишущая в собственном worktree (`isolation = worktree`).
fn is_parallel_eligible(role: &SubagentRoleSpec) -> bool {
    role.parallel_safe || role.isolation == SubagentIsolation::Worktree
}

fn find_role<'a>(roles: &'a [SubagentRoleSpec], call: &ToolCall) -> Option<&'a SubagentRoleSpec> {
    call.args
        .get("agent_type")
        .and_then(Value::as_str)
        .and_then(|name| roles.iter().find(|role| role.name == name))
}

/// Батч task-вызовов можно исполнять конкурентно, только если каждый вызов
/// адресует существующую parallel-eligible роль.
pub fn all_roles_parallel_eligible(calls: &[ToolCall], roles: &[SubagentRoleSpec]) -> bool {
    calls
        .iter()
        .all(|call| find_role(roles, call).is_some_and(is_parallel_eligible))
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

/// Подготовленный запуск: запрос (с подменённым на worktree cwd для
/// изолированных ролей) и его workspace, если роль worktree-изолирована.
struct PreparedTask {
    request: SubagentRequest,
    workspace: Option<WorkspaceInfo>,
}

/// Разбирает вызов и обеспечивает worktree для изолированной роли: fresh —
/// новый worktree от HEAD родительского cwd, resume — рабочая копия того же
/// task_id из реестра. Ошибки — текст для error ToolResult.
fn prepare_task_call(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    call: &ToolCall,
    roles: &[SubagentRoleSpec],
) -> Result<PreparedTask, String> {
    let mut request = parse_task_call(input, call)?;
    let worktree_role =
        find_role(roles, call).is_some_and(|role| role.isolation == SubagentIsolation::Worktree);
    if !worktree_role {
        return Ok(PreparedTask {
            request,
            workspace: None,
        });
    }

    let resume_task_id = request
        .metadata
        .get("task_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let workspace = match resume_task_id {
        Some(task_id) => lookup_workspace(&task_id).ok_or_else(|| {
            format!(
                "worktree for task_id {task_id} is gone (removed as unchanged, merged, or lost on restart); start a fresh task instead"
            )
        })?,
        None => {
            let name = workspace_name(&request.role, &call.id);
            create_subagent_workspace(
                host,
                &SubagentWorkspaceRequest::new(request.task.cwd.clone(), name),
            )
            .map_err(|error| error.message.into_string())?
        }
    };
    request.task.cwd = workspace.path.clone();
    Ok(PreparedTask {
        request,
        workspace: Some(workspace),
    })
}

/// Имя workspace из роли и call id: только [A-Za-z0-9._-], без ведущей точки.
fn workspace_name(role: &str, call_id: &str) -> String {
    let sanitize = |value: &str| {
        value
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>()
    };
    format!("{}-{}", sanitize(role), sanitize(call_id))
        .trim_start_matches(['.', '-'])
        .to_owned()
}

fn child_task_id(result: &SubagentResult) -> Option<String> {
    result.child_thread_id.as_ref().and_then(|child_thread_id| {
        serde_json::to_value(child_thread_id)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
    })
}

/// Пост-обработка workspace после завершения ребёнка: чистый worktree
/// убирается, изменённый — регистрируется за task_id и аннотируется в
/// выводе (merge — обязанность родителя). Возвращает строку-приписку к
/// output ребёнка.
fn finalize_workspace(
    host: &mut PluginWorkflowHostMut<'_>,
    workspace: Option<WorkspaceInfo>,
    result: &SubagentResult,
) -> Option<String> {
    let info = workspace?;
    match cleanup_subagent_workspace(host, &info) {
        Ok(true) => {
            if let Some(task_id) = child_task_id(result) {
                forget_workspace(&task_id);
            }
            Some("[worktree: no changes, removed]".to_owned())
        }
        Ok(false) => {
            if let Some(task_id) = child_task_id(result) {
                register_workspace(task_id, info.clone());
            }
            Some(format!(
                "[worktree: {} | branch: {}]\nThe subagent's changes live only on that branch. Review and merge it yourself, then remove the worktree.",
                info.path.display(),
                info.branch
            ))
        }
        Err(error) => Some(format!(
            "[worktree cleanup failed: {}; worktree left at {}]",
            error.message,
            info.path.display()
        )),
    }
}

/// Error ToolResult для провалившегося запуска: свежесозданный workspace
/// прибирается (чистый — удаляется; тронутый — остаётся, путь дописывается
/// в сообщение об ошибке).
fn error_result_with_workspace(
    host: &mut PluginWorkflowHostMut<'_>,
    call: &ToolCall,
    mut message: String,
    workspace: Option<WorkspaceInfo>,
) -> ToolResult {
    if let Some(info) = workspace {
        match cleanup_subagent_workspace(host, &info) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                message.push_str(&format!(
                    "\n[worktree left at {} | branch: {}]",
                    info.path.display(),
                    info.branch
                ));
            }
        }
    }
    ToolResult::error(call.id.clone(), message).with_metadata(json!({
        "tool": TASK_TOOL,
    }))
}

pub fn handle_task_tool_call(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    call: &ToolCall,
) -> Result<ToolResult, PluginWorkflowError> {
    let roles = subagent_roles(host)?;
    let prepared = match prepare_task_call(host, input, call, &roles) {
        Ok(prepared) => prepared,
        Err(message) => return Ok(ToolResult::error(call.id.clone(), message)),
    };

    match run_subagent(host, &prepared.request) {
        Ok(result) => {
            let note = finalize_workspace(host, prepared.workspace, &result);
            Ok(result_to_tool_result(call, result, note))
        }
        Err(error) => Ok(error_result_with_workspace(
            host,
            call,
            error.message.into_string(),
            prepared.workspace,
        )),
    }
}

/// Конкурентное исполнение батча task-вызовов: сначала spawn всех детей
/// (ToolCallRequested в порядке вызовов), затем wait в том же порядке.
/// Worktree-изолированные роли получают собственный workspace до spawn-а.
/// Ошибка одного вызова (аргументы, workspace, spawn, wait) даёт error
/// ToolResult и не прерывает остальных: уже запущенные дети дожидаются в
/// любом случае.
pub fn handle_parallel_task_calls(
    host: &mut PluginWorkflowHostMut<'_>,
    input: &PluginWorkflowInput,
    calls: &[ToolCall],
    roles: &[SubagentRoleSpec],
) -> Result<Vec<ToolResult>, PluginWorkflowError> {
    enum Spawned {
        Running(SubagentHandle, Option<WorkspaceInfo>),
        Failed(String, Option<WorkspaceInfo>),
    }

    let mut spawned = Vec::with_capacity(calls.len());
    for call in calls {
        emit_event(host, &Event::ToolCallRequested { call: call.clone() })?;
        let outcome = match prepare_task_call(host, input, call, roles) {
            Ok(prepared) => match spawn_subagent(host, &prepared.request) {
                Ok(handle) => Spawned::Running(handle, prepared.workspace),
                Err(error) => Spawned::Failed(error.message.into_string(), prepared.workspace),
            },
            Err(message) => Spawned::Failed(message, None),
        };
        spawned.push(outcome);
    }

    let mut results = Vec::with_capacity(calls.len());
    for (call, outcome) in calls.iter().zip(spawned) {
        let result = match outcome {
            Spawned::Running(handle, workspace) => match wait_subagent(host, &handle) {
                Ok(result) => {
                    let note = finalize_workspace(host, workspace, &result);
                    result_to_tool_result(call, result, note)
                }
                Err(error) => {
                    error_result_with_workspace(host, call, error.message.into_string(), workspace)
                }
            },
            Spawned::Failed(message, workspace) => {
                error_result_with_workspace(host, call, message, workspace)
            }
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

fn result_to_tool_result(
    call: &ToolCall,
    result: SubagentResult,
    workspace_note: Option<String>,
) -> ToolResult {
    let task_id = child_task_id(&result);
    let mut output = result.summary;
    if let Some(task_id) = task_id {
        output.push_str("\n\n[task_id: ");
        output.push_str(&task_id);
        output.push(']');
    }
    if let Some(note) = workspace_note {
        output.push('\n');
        output.push_str(&note);
    }

    ToolResult::ok(call.id.clone(), output).with_metadata(json!({
        "status": result.status,
        "iterations": result.iterations,
        "child_thread_id": result.child_thread_id,
        "usage": result.usage,
    }))
}
