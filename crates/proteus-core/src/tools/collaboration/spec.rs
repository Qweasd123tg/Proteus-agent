use serde_json::json;

use crate::{
    contracts::SubagentRoleSpec,
    domain::{ToolSafety, ToolSpec},
};

fn role_schema(roles: &[SubagentRoleSpec]) -> serde_json::Value {
    let names = roles
        .iter()
        .filter(|role| {
            role.parallel_safe && role.isolation == crate::contracts::SubagentIsolation::None
        })
        .map(|role| role.name.clone())
        .collect::<Vec<_>>();
    json!({
        "type": "string",
        "enum": names,
        "description": "Configured subagent role. Collaboration accepts only parallel_safe roles with isolation=none."
    })
}

pub(super) fn spawn_spec(roles: &[SubagentRoleSpec], timeout_ms: u64) -> ToolSpec {
    ToolSpec::new(
        "spawn_agent",
        "Start one experimental Proteus collaboration agent. Returns a session-owned canonical /root/<task_name> handle immediately.",
        json!({
            "type": "object",
            "properties": {
                "task_name": {
                    "type": "string",
                    "pattern": "^[a-z0-9_]+$",
                    "minLength": 1,
                    "maxLength": 64,
                    "description": "Unique one-segment name within this session."
                },
                "message": { "type": "string", "description": "Task sent to the child agent." },
                "agent_type": role_schema(roles)
            },
            "required": ["task_name", "message", "agent_type"],
            "additionalProperties": false
        }),
        ToolSafety::WritesFiles,
    )
    .with_timeout(timeout_ms)
    .with_metadata(collaboration_metadata())
}

pub(super) fn list_spec(timeout_ms: u64) -> ToolSpec {
    ToolSpec::new(
        "list_agents",
        "List experimental Proteus collaboration agents owned by the current session.",
        json!({
            "type": "object",
            "properties": {
                "path_prefix": { "type": "string", "description": "Optional /root or /root/<task_name> prefix." }
            },
            "additionalProperties": false
        }),
        ToolSafety::ReadOnly,
    )
    .with_timeout(timeout_ms)
    .with_metadata(collaboration_metadata())
}

pub(super) fn wait_spec(timeout_ms: u64) -> ToolSpec {
    ToolSpec::new(
        "wait_agent",
        "Wait for the next queued completion update from this session. A timeout does not consume or cancel updates; a successful wait consumes the returned update batch while list_agents retains durable status.",
        json!({
            "type": "object",
            "properties": {
                "timeout_ms": { "type": "integer", "minimum": 0, "maximum": 300000 }
            },
            "additionalProperties": false
        }),
        ToolSafety::ReadOnly,
    )
    .with_timeout(timeout_ms)
    .with_metadata(collaboration_metadata())
}

pub(super) fn interrupt_spec(timeout_ms: u64) -> ToolSpec {
    ToolSpec::new(
        "interrupt_agent",
        "Request cancellation of one session-owned collaboration agent. Its bounded terminal payload is queued for wait_agent; list_agents retains durable terminal status.",
        json!({
            "type": "object",
            "properties": {
                "target": { "type": "string", "description": "Canonical /root/<task_name> path or task_name." }
            },
            "required": ["target"],
            "additionalProperties": false
        }),
        ToolSafety::ReadOnly,
    )
    .with_timeout(timeout_ms)
    .with_metadata(collaboration_metadata())
}

pub(super) fn send_message_spec(timeout_ms: u64) -> ToolSpec {
    ToolSpec::new(
        "send_message",
        "Queue a message for one running session-owned collaboration agent. Delivery occurs at the nearest model/tool boundary and does not start an idle turn; use followup_task for an idle agent.",
        message_schema(),
        ToolSafety::WritesFiles,
    )
    .with_timeout(timeout_ms)
    .with_metadata(collaboration_metadata())
}

pub(super) fn followup_spec(timeout_ms: u64) -> ToolSpec {
    ToolSpec::new(
        "followup_task",
        "Send a follow-up to one session-owned collaboration agent. Running agents receive it at the nearest model/tool boundary; an idle terminal agent starts a resumable turn with the same logical path and thread id.",
        message_schema(),
        ToolSafety::WritesFiles,
    )
    .with_timeout(timeout_ms)
    .with_metadata(collaboration_metadata())
}

fn message_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "target": { "type": "string", "description": "Canonical /root/<task_name> path or task_name." },
            "message": { "type": "string", "minLength": 1, "maxLength": 16000 }
        },
        "required": ["target", "message"],
        "additionalProperties": false
    })
}

fn collaboration_metadata() -> serde_json::Value {
    json!({
        "hot": true,
        "category": "proteus_subagent_control",
    })
}
