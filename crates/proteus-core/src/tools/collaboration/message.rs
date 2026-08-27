//! Model-facing collaboration messaging tools.
//!
//! `send_message` addresses only a running child mailbox. `followup_task`
//! sends to that mailbox while the child is active, or atomically resumes an
//! idle terminal child by its existing task/thread id. There is no fresh-run
//! fallback when resume is unavailable.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::json;

use super::{
    control::{CollaborationControl, FollowupRequest},
    host, json_result, required_string, session_id, spawn_monitor,
    spec::{followup_spec, send_message_spec},
    tool_error,
};
use crate::{
    contracts::{
        AgentAddress, AgentControlMessage, AgentControlRequest, AgentDeliveryDisposition,
        AgentMessageReceipt, MAX_AGENT_MESSAGE_BYTES, Tool, ToolContext,
    },
    domain::{ToolCall, ToolResult, ToolSpec},
};

pub(super) struct SendMessageTool {
    timeout_ms: u64,
    control: CollaborationControl,
}

impl SendMessageTool {
    pub(super) fn new(timeout_ms: u64, control: CollaborationControl) -> Self {
        Self {
            timeout_ms,
            control,
        }
    }
}

#[async_trait]
impl Tool for SendMessageTool {
    fn spec(&self) -> ToolSpec {
        send_message_spec(self.timeout_ms)
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let current_host = host(&ctx)?;
        let session_id = session_id(&current_host)?;
        let target = required_string(call, "target")?;
        let message = match required_message(call) {
            Ok(message) => message,
            Err(error) => return Ok(tool_error(call, "send_message", error.to_string())),
        };
        let running = match self.control.running_agent(session_id, target) {
            Ok(running) => running,
            Err(error) => return Ok(tool_error(call, "send_message", error.to_string())),
        };
        let target = AgentAddress::parse(&running.path)?;
        let message = match AgentControlMessage::from_root(target.clone(), message) {
            Ok(message) => message,
            Err(error) => return Ok(tool_error(call, "send_message", error.to_string())),
        };
        if let Err(error) = running.owner.send_agent(&running.handle, message).await {
            return Ok(tool_error(call, "send_message", format!("{error:#}")));
        }
        Ok(json_result(
            call,
            "send_message",
            serde_json::to_value(AgentMessageReceipt {
                path: target,
                delivery: AgentDeliveryDisposition::Queued,
                turn_started: false,
                generation: None,
            })?,
        ))
    }
}

pub(super) struct FollowupTaskTool {
    timeout_ms: u64,
    control: CollaborationControl,
}

impl FollowupTaskTool {
    pub(super) fn new(timeout_ms: u64, control: CollaborationControl) -> Self {
        Self {
            timeout_ms,
            control,
        }
    }
}

#[async_trait]
impl Tool for FollowupTaskTool {
    fn spec(&self) -> ToolSpec {
        followup_spec(self.timeout_ms)
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        let current_host = host(&ctx)?;
        let session_id = session_id(&current_host)?;
        let target = required_string(call, "target")?;
        let message = match required_message(call) {
            Ok(message) => message,
            Err(error) => return Ok(tool_error(call, "followup_task", error.to_string())),
        };
        let followup = match self.control.begin_followup(session_id, target) {
            Ok(followup) => followup,
            Err(error) => return Ok(tool_error(call, "followup_task", error.to_string())),
        };

        match followup {
            FollowupRequest::Running(running) => {
                let target = AgentAddress::parse(&running.path)?;
                let message = match AgentControlMessage::from_root(target.clone(), message) {
                    Ok(message) => message,
                    Err(error) => {
                        return Ok(tool_error(call, "followup_task", error.to_string()));
                    }
                };
                if let Err(error) = running.owner.send_agent(&running.handle, message).await {
                    return Ok(tool_error(call, "followup_task", format!("{error:#}")));
                }
                Ok(json_result(
                    call,
                    "followup_task",
                    serde_json::to_value(AgentMessageReceipt {
                        path: target,
                        delivery: AgentDeliveryDisposition::Queued,
                        turn_started: false,
                        generation: None,
                    })?,
                ))
            }
            FollowupRequest::Idle(idle) => {
                let Some(parent_task) = ctx.task.clone() else {
                    self.control
                        .abort_followup(session_id, &idle.path, idle.generation);
                    return Ok(tool_error(
                        call,
                        "followup_task",
                        anyhow!("followup_task requires current AgentTask").to_string(),
                    ));
                };
                let target = AgentAddress::parse(&idle.path)?;
                let message = match AgentControlMessage::from_root(target.clone(), message) {
                    Ok(message) => message,
                    Err(error) => {
                        self.control
                            .abort_followup(session_id, &idle.path, idle.generation);
                        return Ok(tool_error(call, "followup_task", error.to_string()));
                    }
                };
                let request =
                    AgentControlRequest::new(&idle.role, message.model_text(), parent_task)
                        .with_description(idle.task_name.clone())
                        .with_metadata(json!({
                            "control_plane_owned": true,
                            "agent_control_target": idle.path,
                            "task_id": idle.task_id,
                            "agent_control_message": message,
                        }));
                let handle = match current_host.spawn_agent(request).await {
                    Ok(handle) => handle,
                    Err(error) => {
                        self.control
                            .abort_followup(session_id, &idle.path, idle.generation);
                        return Ok(tool_error(call, "followup_task", format!("{error:#}")));
                    }
                };
                let interrupt_requested = match self.control.attach_followup(
                    session_id,
                    &idle,
                    handle.clone(),
                    current_host.clone(),
                ) {
                    Ok(interrupt_requested) => interrupt_requested,
                    Err(error) => {
                        let _ = current_host.cancel_agent(&handle).await;
                        self.control
                            .abort_followup(session_id, &idle.path, idle.generation);
                        return Ok(tool_error(call, "followup_task", error.to_string()));
                    }
                };
                spawn_monitor(
                    self.control.clone(),
                    current_host.clone(),
                    session_id,
                    idle.path.clone(),
                    idle.generation,
                    handle.clone(),
                );
                if interrupt_requested && let Err(error) = current_host.cancel_agent(&handle).await
                {
                    return Ok(tool_error(call, "followup_task", format!("{error:#}")));
                }
                Ok(json_result(
                    call,
                    "followup_task",
                    serde_json::to_value(AgentMessageReceipt {
                        path: target,
                        delivery: AgentDeliveryDisposition::Resumed,
                        turn_started: true,
                        generation: Some(idle.generation),
                    })?,
                ))
            }
        }
    }
}

fn required_message(call: &ToolCall) -> Result<&str> {
    let message = required_string(call, "message")?;
    if message.trim().is_empty() {
        return Err(anyhow!("{} arg 'message' must not be blank", call.name));
    }
    if message.len() > MAX_AGENT_MESSAGE_BYTES {
        return Err(anyhow!(
            "{} arg 'message' exceeds {MAX_AGENT_MESSAGE_BYTES} bytes",
            call.name
        ));
    }
    Ok(message)
}
