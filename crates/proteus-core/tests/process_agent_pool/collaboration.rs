//! Real process reuse through the registered collaboration facade.

use async_trait::async_trait;
use proteus_contracts::{
    contracts::{
        AgentControl, AgentControlHandle, AgentControlMessage, AgentControlRequest,
        AgentControlResult, AgentControlToolHost, AgentListSnapshot, AgentWaitSnapshot,
        AgentWorkflowContext, ExecutionAttribution, ToolContext, ToolRegistry,
    },
    domain::{AgentTask, SessionId, ToolCall, ToolResult, new_call_id, new_execution_id},
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};

use super::{
    AgentControlConfig, AgentControlRuntime, InMemoryEventStore, proteus_binary,
    test_runtime_context,
};

struct ProcessHost {
    service: Arc<dyn AgentControl>,
    ctx: AgentWorkflowContext,
}

#[async_trait]
impl AgentControlToolHost for ProcessHost {
    fn session_id(&self) -> Option<SessionId> {
        Some(self.ctx.session_id)
    }

    async fn run_agent(&self, request: AgentControlRequest) -> anyhow::Result<AgentControlResult> {
        self.service.run(request, self.ctx.clone()).await
    }

    async fn spawn_agent(
        &self,
        request: AgentControlRequest,
    ) -> anyhow::Result<AgentControlHandle> {
        self.service.spawn(request, self.ctx.clone()).await
    }

    async fn wait_agent(&self, handle: &AgentControlHandle) -> anyhow::Result<AgentControlResult> {
        self.service.wait(handle).await
    }

    async fn cancel_agent(&self, handle: &AgentControlHandle) -> anyhow::Result<()> {
        self.service.cancel(handle).await
    }

    async fn send_agent(
        &self,
        handle: &AgentControlHandle,
        message: AgentControlMessage,
    ) -> anyhow::Result<()> {
        self.service.send(handle, message).await
    }
}

async fn invoke(tools: &ToolRegistry, ctx: &ToolContext, name: &str, args: Value) -> ToolResult {
    tools
        .get(name)
        .expect("registered collaboration tool")
        .invoke(&ToolCall::new(new_call_id(), name, args), ctx.clone())
        .await
        .expect("invoke tool")
}

async fn wait(tools: &ToolRegistry, ctx: &ToolContext) -> AgentWaitSnapshot {
    let result = invoke(tools, ctx, "wait_agent", json!({"timeout_ms":60_000})).await;
    assert!(result.ok, "{result:?}");
    let snapshot: AgentWaitSnapshot = serde_json::from_str(&result.output).expect("wait snapshot");
    assert!(!snapshot.timed_out, "completion must be delivered");
    assert_eq!(snapshot.agents.len(), 1);
    snapshot
}

pub(super) async fn fresh_task_expires_facade_history(config_path: &Path, cwd: &Path) {
    let config: AgentControlConfig = serde_json::from_value(json!({
        "binary":proteus_binary(), "surface":"collaboration", "max_idle_processes":8,
        "roles":[{"name":"helper", "description":"Stub helper child", "config":config_path,
                  "parallel_safe":true, "max_processes":4, "timeout_ms":60_000}]
    }))
    .expect("config");
    let runtime = AgentControlRuntime::from_config(&config).expect("runtime");
    let runner = runtime.service().expect("service");
    let mut tools = ToolRegistry::new();
    runtime
        .register_tools(&mut tools, 60_000)
        .expect("register facade");
    let events = Arc::new(InMemoryEventStore::new());
    let runtime_ctx = test_runtime_context(events.clone());
    let task = AgentTask::new("delegate", cwd.to_path_buf());
    let mut ctx = ToolContext::new(
        cwd.to_path_buf(),
        ExecutionAttribution::detached(new_execution_id()),
    );
    ctx.task = Some(task.clone());
    ctx.agent_control = Some(Arc::new(ProcessHost {
        service: runner.clone(),
        ctx: runtime_ctx.clone(),
    }));

    let first = invoke(
        &tools,
        &ctx,
        "spawn_agent",
        json!({"task_name":"first", "message":"first task", "agent_type":"helper"}),
    )
    .await;
    assert!(first.ok, "{first:?}");
    let first = wait(&tools, &ctx).await;
    let first_task_id = first.agents[0]
        .child_thread_id
        .expect("first is initially resumable");

    // Same role/cwd reuse happens even with spare role and idle capacity.
    let fresh = invoke(
        &tools,
        &ctx,
        "spawn_agent",
        json!({"task_name":"fresh", "message":"fresh task", "agent_type":"helper"}),
    )
    .await;
    assert!(fresh.ok, "{fresh:?}");
    let fresh = wait(&tools, &ctx).await;
    let fresh_task_id = fresh.agents[0]
        .child_thread_id
        .expect("fresh task remains resumable");

    let listed = invoke(&tools, &ctx, "list_agents", json!({"path_prefix":"first"})).await;
    assert!(listed.ok, "{listed:?}");
    let listed: AgentListSnapshot = serde_json::from_str(&listed.output).expect("list snapshot");
    assert_eq!(listed.agents.len(), 1);
    assert_eq!(listed.agents[0].child_thread_id, None);
    assert_eq!(
        listed.agents[0].status,
        super::AgentLifecycleStatus::Completed
    );
    let events_before_followup = events.envelopes().await.len();
    let rejected = invoke(
        &tools,
        &ctx,
        "followup_task",
        json!({"target":"first", "message":"continue first"}),
    )
    .await;
    assert!(!rejected.ok);
    assert_eq!(
        rejected.error.as_deref(),
        Some("collaboration agent '/root/first' has no resumable task id")
    );
    assert_eq!(
        events.envelopes().await.len(),
        events_before_followup,
        "expired follow-up must not dispatch"
    );

    // The backend still rejects a raw obsolete task_id; the facade rejects
    // earlier with a coherent lifecycle error rather than forwarding this.
    let stale = runner
        .run(
            AgentControlRequest::new("helper", "continue first", task)
                .with_metadata(json!({"task_id":first_task_id.to_string()})),
            runtime_ctx,
        )
        .await
        .expect_err("expired binding");
    assert!(stale.to_string().contains("unknown task_id"), "{stale:#}");

    let resumed = invoke(
        &tools,
        &ctx,
        "followup_task",
        json!({"target":"fresh", "message":"continue fresh"}),
    )
    .await;
    assert!(resumed.ok, "{resumed:?}");
    let resumed = wait(&tools, &ctx).await;
    assert_eq!(resumed.agents[0].child_thread_id, Some(fresh_task_id));
    assert_eq!(resumed.agents[0].generation, 2);
}
