//! Process-backed subagent runner and its root-owned lifecycle primitives.
//!
//! A child is always another complete Proteus process with its own config,
//! runtime, model, tools and policy. This module only prepares an isolated
//! runtime context for event attribution/cancellation and owns the local
//! process connection, mailbox and pending-operation state.

mod mailbox;
mod pending;
mod process;

pub use process::ProcessAgentControl;

use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::{
    contracts::{AgentAddress, AgentControlRequest, RuntimeContext},
    domain::ThreadId,
};

fn requested_agent_target(request: &AgentControlRequest) -> Result<Option<AgentAddress>> {
    let control_plane_owned = request
        .metadata
        .get("control_plane_owned")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let target = request
        .metadata
        .get("agent_control_target")
        .and_then(Value::as_str);
    match (control_plane_owned, target) {
        (true, Some(target)) => AgentAddress::parse(target).map(Some),
        (true, None) => bail!("collaboration-owned subagent requires agent_control_target"),
        (false, Some(_)) => bail!("agent_control_target requires control_plane_owned=true"),
        (false, None) => Ok(None),
    }
}

/// Контекст process peer-а поверх родительского: собственный `thread_id`,
/// role label для attribution, пустые turn-scoped grants и отдельный child
/// cancellation token. Parent cancellation каскадируется ребёнку, targeted
/// cancel ребёнка не затрагивает parent turn и соседние процессы.
fn child_context(
    ctx: &RuntimeContext,
    child_thread_id: ThreadId,
    role_name: &str,
) -> RuntimeContext {
    let mut child_ctx = ctx.clone();
    child_ctx.thread_id = child_thread_id;
    child_ctx.thread_label = Some(role_name.to_owned());
    child_ctx.turn_grants = Arc::default();
    child_ctx.cancellation = ctx.cancellation.child_token();
    child_ctx
}
