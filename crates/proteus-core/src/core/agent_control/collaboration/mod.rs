//! Experimental Codex-shaped collaboration facade over root-owned
//! `AgentControl`. This is a bounded session control plane, not a parity claim.

mod control;
mod message;
mod spec;

#[cfg(test)]
mod tests;

use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    contracts::{
        AGENT_CONTROL_SCHEMA_VERSION, AgentAddress, AgentControlHandle, AgentControlRequest,
        AgentControlToolHost, AgentInterruptReceipt, AgentInterruptStatus, AgentIsolation,
        AgentLifecycleStatus, AgentListSnapshot, AgentProfile, AgentSpawnReceipt,
        AgentWaitSnapshot, Tool, ToolContext, ToolRegistry, ToolSource,
    },
    domain::{SessionId, ToolCall, ToolResult, ToolSpec},
};

use control::CollaborationControl;
use message::{FollowupTaskTool, SendMessageTool};
use spec::{interrupt_spec, list_spec, spawn_spec, wait_spec};

#[cfg(test)]
pub(super) const COLLABORATION_TOOL_NAMES: &[&str] = &[
    "spawn_agent",
    "list_agents",
    "wait_agent",
    "interrupt_agent",
    "send_message",
    "followup_task",
];

const DEFAULT_WAIT_MS: u64 = 30_000;
const MAX_WAIT_MS: u64 = 300_000;

pub(super) fn register_collaboration_tools(
    tools: &mut ToolRegistry,
    roles: Vec<AgentProfile>,
    timeout_ms: u64,
) -> Result<()> {
    // Process runtime service: registry/config rebuilds receive the same
    // bounded session-owned control plane, so live handles are not orphaned.
    let control = CollaborationControl::shared();
    let source = ToolSource::builtin("agent-control-collaboration");
    tools.register_with_source(
        source.clone(),
        SpawnAgentTool::new(roles, timeout_ms, control.clone()),
    )?;
    tools.register_with_source(
        source.clone(),
        ListAgentsTool::new(timeout_ms, control.clone()),
    )?;
    tools.register_with_source(
        source.clone(),
        WaitAgentTool::new(timeout_ms, control.clone()),
    )?;
    tools.register_with_source(
        source.clone(),
        InterruptAgentTool::new(timeout_ms, control.clone()),
    )?;
    tools.register_with_source(
        source.clone(),
        SendMessageTool::new(timeout_ms, control.clone()),
    )?;
    tools.register_with_source(source, FollowupTaskTool::new(timeout_ms, control))?;
    Ok(())
}

struct SpawnAgentTool {
    roles: Vec<AgentProfile>,
    timeout_ms: u64,
    control: CollaborationControl,
}

impl SpawnAgentTool {
    fn new(roles: Vec<AgentProfile>, timeout_ms: u64, control: CollaborationControl) -> Self {
        Self {
            roles,
            timeout_ms,
            control,
        }
    }
}

#[async_trait]
impl Tool for SpawnAgentTool {
    fn spec(&self) -> ToolSpec {
        spawn_spec(&self.roles, self.timeout_ms)
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let host = host(&ctx)?;
        let session_id = session_id(&host)?;
        let parent_task = ctx
            .task
            .clone()
            .ok_or_else(|| anyhow!("spawn_agent requires current AgentTask"))?;
        let task_name = required_string(call, "task_name")?;
        let message = required_string(call, "message")?;
        let agent_type = required_string(call, "agent_type")?;
        if message.trim().is_empty() {
            return Ok(tool_error(
                call,
                "spawn_agent",
                "spawn_agent arg 'message' must not be blank".to_owned(),
            ));
        }
        let role = self
            .roles
            .iter()
            .find(|role| role.name == agent_type)
            .ok_or_else(|| anyhow!("unknown agent profile '{agent_type}'"))?;
        if role.isolation != AgentIsolation::None {
            return Ok(tool_error(
                call,
                "spawn_agent",
                format!(
                    "role '{agent_type}' uses {:?} isolation; collaboration currently supports isolation=none only (use surface=task for writer/worktree roles)",
                    role.isolation
                ),
            ));
        }
        if !role.parallel_safe {
            return Ok(tool_error(
                call,
                "spawn_agent",
                format!(
                    "role '{agent_type}' is not parallel_safe; collaboration accepts only explicitly parallel-safe read-only roles (use surface=task for this role)"
                ),
            ));
        }

        let reservation = match self.control.reserve(session_id, task_name, agent_type) {
            Ok(reservation) => reservation,
            Err(error) => return Ok(tool_error(call, "spawn_agent", error.to_string())),
        };
        let request = AgentControlRequest::new(agent_type, message, parent_task)
            .with_description(task_name.to_owned())
            .with_metadata(json!({
                "control_plane_owned": true,
                "agent_control_target": reservation.path,
            }));
        let handle = match host.spawn_agent(request).await {
            Ok(handle) => handle,
            Err(error) => {
                self.control
                    .release_reservation(session_id, &reservation.path);
                return Ok(tool_error(call, "spawn_agent", format!("{error:#}")));
            }
        };
        let interrupt_requested = self.control.attach(
            session_id,
            &reservation.path,
            reservation.generation,
            handle.clone(),
            host.clone(),
        )?;
        spawn_monitor(
            self.control.clone(),
            host.clone(),
            session_id,
            reservation.path.clone(),
            reservation.generation,
            handle.clone(),
        );
        if interrupt_requested {
            host.cancel_agent(&handle).await?;
        }

        let receipt = AgentSpawnReceipt {
            schema_version: AGENT_CONTROL_SCHEMA_VERSION,
            path: AgentAddress::parse(&reservation.path)?,
            generation: reservation.generation,
            task_name: task_name.to_owned(),
            agent_type: agent_type.to_owned(),
            status: AgentLifecycleStatus::Running,
        };
        Ok(
            ToolResult::ok(call.id.clone(), serde_json::to_string(&receipt)?)
                .with_metadata(json!({ "tool": "spawn_agent", "path": reservation.path })),
        )
    }
}

struct ListAgentsTool {
    timeout_ms: u64,
    control: CollaborationControl,
}

impl ListAgentsTool {
    fn new(timeout_ms: u64, control: CollaborationControl) -> Self {
        Self {
            timeout_ms,
            control,
        }
    }
}

#[async_trait]
impl Tool for ListAgentsTool {
    fn spec(&self) -> ToolSpec {
        list_spec(self.timeout_ms)
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let host = host(&ctx)?;
        let session_id = session_id(&host)?;
        let prefix = optional_string(call, "path_prefix")?;
        match self.control.list(session_id, prefix) {
            Ok(agents) => Ok(json_result(
                call,
                "list_agents",
                serde_json::to_value(AgentListSnapshot { agents })?,
            )),
            Err(error) => Ok(tool_error(call, "list_agents", error.to_string())),
        }
    }
}

struct WaitAgentTool {
    timeout_ms: u64,
    control: CollaborationControl,
}

impl WaitAgentTool {
    fn new(timeout_ms: u64, control: CollaborationControl) -> Self {
        Self {
            timeout_ms,
            control,
        }
    }
}

#[async_trait]
impl Tool for WaitAgentTool {
    fn spec(&self) -> ToolSpec {
        wait_spec(self.timeout_ms)
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let host = host(&ctx)?;
        let session_id = session_id(&host)?;
        let timeout_ms = optional_u64(call, "timeout_ms")?.unwrap_or(DEFAULT_WAIT_MS);
        if timeout_ms > MAX_WAIT_MS {
            return Ok(tool_error(
                call,
                "wait_agent",
                format!("timeout_ms must be <= {MAX_WAIT_MS}"),
            ));
        }
        let notify = self.control.session_notify(session_id)?;
        let completed = if self.control.has_completions(session_id)? {
            self.control.drain_completions(session_id)?
        } else if timeout_ms == 0 {
            Vec::new()
        } else {
            let control = self.control.clone();
            tokio::time::timeout(Duration::from_millis(timeout_ms), async move {
                loop {
                    let notified = notify.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    if control.has_completions(session_id)? {
                        return control.drain_completions(session_id);
                    }
                    notified.as_mut().await;
                    // Another concurrent waiter can drain the queue after the
                    // broadcast wake. Loop until our own timeout or a later
                    // completion instead of returning an empty false-success.
                }
            })
            .await
            .unwrap_or_else(|_| Ok(Vec::new()))?
        };
        let timed_out = completed.is_empty();
        Ok(json_result(
            call,
            "wait_agent",
            serde_json::to_value(AgentWaitSnapshot {
                timed_out,
                agents: completed,
            })?,
        ))
    }
}

struct InterruptAgentTool {
    timeout_ms: u64,
    control: CollaborationControl,
}

impl InterruptAgentTool {
    fn new(timeout_ms: u64, control: CollaborationControl) -> Self {
        Self {
            timeout_ms,
            control,
        }
    }
}

#[async_trait]
impl Tool for InterruptAgentTool {
    fn spec(&self) -> ToolSpec {
        interrupt_spec(self.timeout_ms)
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let host = host(&ctx)?;
        let session_id = session_id(&host)?;
        let target = required_string(call, "target")?;
        let interrupt = match self.control.request_interrupt(session_id, target) {
            Ok(state) => state,
            Err(error) => return Ok(tool_error(call, "interrupt_agent", error.to_string())),
        };
        if !interrupt.terminal
            && let Some((owner, handle)) = interrupt.owned_handle
            && let Err(error) = owner.cancel_agent(&handle).await
        {
            return Ok(tool_error(call, "interrupt_agent", format!("{error:#}")));
        }
        Ok(json_result(
            call,
            "interrupt_agent",
            serde_json::to_value(AgentInterruptReceipt {
                path: AgentAddress::parse(&interrupt.path)?,
                status: if interrupt.terminal {
                    AgentInterruptStatus::AlreadyTerminal
                } else {
                    AgentInterruptStatus::InterruptRequested
                },
            })?,
        ))
    }
}

fn spawn_monitor(
    control: CollaborationControl,
    host: Arc<dyn AgentControlToolHost>,
    session_id: SessionId,
    path: String,
    generation: u64,
    handle: AgentControlHandle,
) {
    tokio::spawn(async move {
        let result = host.wait_agent(&handle).await;
        control.complete(session_id, &path, generation, result);
    });
}

fn host(ctx: &ToolContext) -> Result<Arc<dyn AgentControlToolHost>> {
    ctx.agent_control
        .clone()
        .ok_or_else(|| anyhow!("collaboration tool requires AgentControlToolHost capability"))
}

fn session_id(host: &Arc<dyn AgentControlToolHost>) -> Result<SessionId> {
    host.session_id()
        .ok_or_else(|| anyhow!("collaboration tool requires a session-bound runtime host"))
}

fn required_string<'a>(call: &'a ToolCall, name: &str) -> Result<&'a str> {
    call.args
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("{} requires string arg '{name}'", call.name))
}

fn optional_string<'a>(call: &'a ToolCall, name: &str) -> Result<Option<&'a str>> {
    match call.args.get(name) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(anyhow!("{} arg '{name}' must be a string", call.name)),
    }
}

fn optional_u64(call: &ToolCall, name: &str) -> Result<Option<u64>> {
    match call.args.get(name) {
        None => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| anyhow!("{} arg '{name}' must be a non-negative integer", call.name)),
        Some(_) => Err(anyhow!(
            "{} arg '{name}' must be a non-negative integer",
            call.name
        )),
    }
}

fn json_result(call: &ToolCall, tool: &str, value: Value) -> ToolResult {
    ToolResult::ok(call.id.clone(), value.to_string()).with_metadata(json!({ "tool": tool }))
}

fn tool_error(call: &ToolCall, tool: &str, message: String) -> ToolResult {
    ToolResult::error(call.id.clone(), message).with_metadata(json!({ "tool": tool }))
}
