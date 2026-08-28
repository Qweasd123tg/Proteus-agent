//! Root-owned agent-control facade and its process lifecycle implementation.
//!
//! A child is always another complete Proteus process with its own config,
//! runtime, model, tools and policy. External core consumers see only
//! [`AgentControlRuntime`]: process connection, pool, mailbox, pending state,
//! per-invocation binding and model-facing tool facades remain private here.

mod collaboration;
mod mailbox;
mod pending;
mod process;
mod task;
mod tool_host;

use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::{
    contracts::{
        AgentAddress, AgentControl, AgentControlRequest, AgentWorkflowContext, CancellationToken,
        ToolRegistry,
    },
    domain::ThreadId,
};

use super::{AgentControlConfig, AgentControlSurface};
use process::ProcessAgentControl;

pub(crate) use task::{TASK_TOOL, calls_are_parallel_eligible};

/// Единственная core-facing точка сборки process agent control и его
/// model-facing facade. Внешние слои не знают process pool, mailbox или
/// child-config representation.
#[derive(Clone)]
pub struct AgentControlRuntime {
    service: Option<Arc<dyn AgentControl>>,
    surface: AgentControlSurface,
}

impl AgentControlRuntime {
    pub fn from_config(config: &AgentControlConfig) -> Result<Self> {
        let service = if config.roles.is_empty() {
            None
        } else {
            Some(Arc::new(ProcessAgentControl::from_config(config.clone())?)
                as Arc<dyn AgentControl>)
        };
        Ok(Self {
            service,
            surface: config.surface,
        })
    }

    pub fn disabled() -> Self {
        Self {
            service: None,
            surface: AgentControlSurface::None,
        }
    }

    pub fn service(&self) -> Option<Arc<dyn AgentControl>> {
        self.service.clone()
    }

    /// Регистрирует ровно одну configured facade поверх того же service,
    /// который будет помещён в `AgentWorkflowContext`.
    pub fn register_tools(&self, tools: &mut ToolRegistry, timeout_ms: u64) -> Result<()> {
        let Some(service) = self.service.as_ref() else {
            return Ok(());
        };
        match self.surface {
            AgentControlSurface::Task => {
                task::register_task_tool(tools, service.profiles(), timeout_ms)
            }
            AgentControlSurface::Collaboration => {
                collaboration::register_collaboration_tools(tools, service.profiles(), timeout_ms)
            }
            AgentControlSurface::None => Ok(()),
        }
    }
}

pub(crate) fn bind_tool_host(
    ctx: &AgentWorkflowContext,
    cancellation: CancellationToken,
) -> Option<Arc<dyn crate::contracts::AgentControlToolHost>> {
    tool_host::bind(ctx, cancellation)
}

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
        (true, None) => bail!("collaboration-owned agent requires agent_control_target"),
        (false, Some(_)) => bail!("agent_control_target requires control_plane_owned=true"),
        (false, None) => Ok(None),
    }
}

/// Контекст process peer-а поверх родительского: собственный `thread_id`,
/// role label для attribution, пустые turn-scoped grants и отдельный child
/// cancellation token. Parent cancellation каскадируется ребёнку, targeted
/// cancel ребёнка не затрагивает parent turn и соседние процессы.
fn child_context(
    ctx: &AgentWorkflowContext,
    child_thread_id: ThreadId,
    role_name: &str,
) -> AgentWorkflowContext {
    let mut child_ctx = ctx.clone();
    child_ctx.thread_id = child_thread_id;
    child_ctx.thread_label = Some(role_name.to_owned());
    child_ctx.turn_grants = Arc::default();
    child_ctx.execution.scope = ctx.execution.scope.child_cancellation_scope();
    child_ctx
}
