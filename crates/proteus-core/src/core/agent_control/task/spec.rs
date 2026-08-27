use serde_json::json;

use crate::{
    contracts::{AgentIsolation, AgentProfile},
    domain::{ToolSafety, ToolSpec},
};

pub(super) fn task_tool_spec(roles: &[AgentProfile], timeout_ms: u64) -> ToolSpec {
    let role_description = roles
        .iter()
        .map(|role| {
            let mut markers = Vec::new();
            if role.parallel_safe {
                markers.push("parallel-safe");
            }
            if role.isolation == AgentIsolation::Worktree {
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
        "Several task calls issued in one reply run concurrently when every requested role is parallel-safe or worktree-isolated (marked in the role list); use that for independent research or independent isolated changes. Any other combination runs sequentially."
    } else {
        "Tasks run sequentially; start another task only after the current delegated work has returned."
    };
    let worktree_line = if roles
        .iter()
        .any(|role| role.isolation == AgentIsolation::Worktree)
    {
        "\nWorktree-isolated roles work in their own git worktree branched from the current HEAD. NOTHING is merged automatically: review and merge the reported branch yourself, then remove the worktree."
    } else {
        ""
    };
    let description = format!(
        "Delegate a focused task to an isolated Proteus agent profile and return its summary.\n\
The subagent starts with a FRESH context, so include all necessary background in the prompt; parent history is not passed.\n\
Reuse task_id from a previous task result to continue that subagent with its accumulated context.\n\
Choose the role that best matches the delegated work and keep the prompt specific.\n\
Do the work yourself when it needs your accumulated context, close supervision, or tight iteration with the user.\n\
{concurrency_line}{worktree_line}"
    );

    ToolSpec::new(
        super::TASK_TOOL,
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
                    "description": "Resume a previous subagent task with its accumulated context."
                }
            },
            "required": ["prompt", "agent_type"]
        }),
        ToolSafety::WritesFiles,
    )
    .with_timeout(timeout_ms)
    .with_metadata(json!({
        "category": "proteus_subagent",
        "hot": true,
    }))
}

pub(super) fn is_parallel_eligible(role: &AgentProfile) -> bool {
    role.parallel_safe || role.isolation == AgentIsolation::Worktree
}
