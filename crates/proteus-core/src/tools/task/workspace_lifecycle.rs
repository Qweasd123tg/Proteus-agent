use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::{
    contracts::{
        SubagentIsolation, SubagentRequest, SubagentResult, SubagentRoleSpec, WorkspaceInfo,
    },
    core::workspace as git_workspace,
    domain::{ToolCall, ToolResult},
};

pub(super) async fn prepare_workspace(
    request: &mut SubagentRequest,
    call: &ToolCall,
    role: &SubagentRoleSpec,
) -> Result<Option<WorkspaceInfo>, String> {
    if role.isolation != SubagentIsolation::Worktree {
        return Ok(None);
    }
    let resume_task_id = request
        .metadata
        .get("task_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let info = match resume_task_id {
        Some(task_id) => workspaces()
            .lock()
            .ok()
            .and_then(|map| map.get(&task_id).cloned())
            .ok_or_else(|| {
                format!(
                    "worktree for task_id {task_id} is gone (removed, merged, or lost on restart); start a fresh task instead"
                )
            })?,
        None => {
            let parent_cwd = request.task.cwd.clone();
            let name = workspace_name(&request.role, &call.id);
            tokio::task::spawn_blocking(move || {
                git_workspace::create_worktree(&parent_cwd, &name)
            })
            .await
            .map_err(|error| format!("worktree worker failed: {error}"))?
            .map_err(|error| format!("{error:#}"))?
        }
    };
    request.task.cwd = info.path.clone();
    Ok(Some(info))
}

pub(super) async fn finalize_workspace(
    info: Option<WorkspaceInfo>,
    result: &SubagentResult,
) -> Option<String> {
    let info = info?;
    match cleanup_workspace(info.clone()).await {
        Ok((_, true)) => {
            if let Some(task_id) = child_task_id(result)
                && let Ok(mut map) = workspaces().lock()
            {
                map.remove(&task_id);
            }
            Some("[worktree: no changes, removed]".to_owned())
        }
        Ok((info, false)) => {
            if let Some(task_id) = child_task_id(result)
                && let Ok(mut map) = workspaces().lock()
            {
                map.insert(task_id, info.clone());
            }
            Some(format!(
                "[worktree: {} | branch: {}]\nThe subagent's changes live only on that branch. Review and merge it yourself, then remove the worktree.",
                info.path.display(),
                info.branch
            ))
        }
        Err(error) => Some(format!(
            "[worktree cleanup failed: {error:#}; worktree left at {}]",
            info.path.display()
        )),
    }
}

pub(super) async fn error_result_with_workspace(
    call: &ToolCall,
    mut message: String,
    info: Option<WorkspaceInfo>,
) -> ToolResult {
    if let Some(info) = info {
        match cleanup_workspace(info.clone()).await {
            Ok((_, true)) => {}
            Ok((info, false)) => message.push_str(&format!(
                "\n[worktree left at {} | branch: {}]",
                info.path.display(),
                info.branch
            )),
            Err(_) => message.push_str(&format!(
                "\n[worktree left at {} | branch: {}]",
                info.path.display(),
                info.branch
            )),
        }
    }
    super::task_error(call, message)
}

fn workspaces() -> &'static Mutex<HashMap<String, WorkspaceInfo>> {
    static WORKSPACES: OnceLock<Mutex<HashMap<String, WorkspaceInfo>>> = OnceLock::new();
    WORKSPACES.get_or_init(Mutex::default)
}

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

pub(super) fn child_task_id(result: &SubagentResult) -> Option<String> {
    result.child_thread_id.map(|id| id.to_string())
}

async fn cleanup_workspace(info: WorkspaceInfo) -> Result<(WorkspaceInfo, bool)> {
    tokio::task::spawn_blocking(move || {
        let removed = git_workspace::cleanup_worktree_if_unchanged(&info)?;
        Ok((info, removed))
    })
    .await
    .map_err(|error| anyhow!("worktree cleanup worker failed: {error}"))?
}
