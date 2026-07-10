use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::*;
use crate::{
    contracts::{CancellationToken, SubagentHandle, SubagentResult, SubagentStatus},
    domain::{AgentTask, ToolCall, new_call_id, new_session_id, new_thread_id},
};

struct TestHost {
    session_id: SessionId,
    finished: CancellationToken,
    cancelled: CancellationToken,
    requests: Mutex<Vec<SubagentRequest>>,
}

impl TestHost {
    fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            finished: CancellationToken::new(),
            cancelled: CancellationToken::new(),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl SubagentToolHost for TestHost {
    fn session_id(&self) -> Option<SessionId> {
        Some(self.session_id)
    }

    async fn run_subagent(&self, _request: SubagentRequest) -> Result<SubagentResult> {
        unreachable!("collaboration tools use spawn")
    }

    async fn spawn_subagent(&self, request: SubagentRequest) -> Result<SubagentHandle> {
        self.requests.lock().unwrap().push(request);
        Ok(SubagentHandle::new(
            new_call_id(),
            "explore",
            new_thread_id(),
        ))
    }

    async fn wait_subagent(&self, handle: &SubagentHandle) -> Result<SubagentResult> {
        tokio::select! {
            _ = self.cancelled.cancelled() => Ok(SubagentResult::new(
                "partial",
                SubagentStatus::Cancelled,
                1,
            ).with_child_thread_id(handle.child_thread_id)),
            _ = self.finished.cancelled() => Ok(SubagentResult::new(
                "done",
                SubagentStatus::Completed,
                2,
            ).with_child_thread_id(handle.child_thread_id)),
        }
    }

    async fn cancel_subagent(&self, _handle: &SubagentHandle) -> Result<()> {
        self.cancelled.cancel();
        Ok(())
    }
}

fn role() -> SubagentRoleSpec {
    SubagentRoleSpec::new("explore", "read only", "inspect").with_parallel_safe(true)
}

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall::new(new_call_id(), name, args)
}

fn context(host: Arc<TestHost>) -> ToolContext {
    let mut ctx = ToolContext::new(std::env::current_dir().expect("cwd"));
    ctx.task = Some(AgentTask::new("parent", ctx.cwd.clone()));
    ctx.subagent = Some(host);
    ctx
}

fn output(result: &ToolResult) -> Value {
    serde_json::from_str(&result.output).expect("json output")
}

#[tokio::test]
async fn timeout_preserves_completion_and_later_waits_poll_the_next_update() {
    let control = CollaborationControl::default();
    let host = Arc::new(TestHost::new(new_session_id()));
    let spawn = SpawnAgentTool::new(vec![role()], 10_000, control.clone());
    let wait = WaitAgentTool::new(10_000, control);
    let ctx = context(host.clone());

    let spawned = spawn
        .invoke(
            &call(
                "spawn_agent",
                json!({"task_name":"scan","message":"inspect","agent_type":"explore"}),
            ),
            ctx.clone(),
        )
        .await
        .expect("spawn");
    assert_eq!(output(&spawned)["path"], "/root/scan");

    let timed_out = wait
        .invoke(&call("wait_agent", json!({"timeout_ms":0})), ctx.clone())
        .await
        .expect("poll");
    assert_eq!(output(&timed_out)["timed_out"], true);

    host.finished.cancel();
    let completed = wait
        .invoke(&call("wait_agent", json!({"timeout_ms":1000})), ctx.clone())
        .await
        .expect("wait completion");
    assert_eq!(output(&completed)["agents"][0]["status"], "completed");

    let repeated = wait
        .invoke(&call("wait_agent", json!({"timeout_ms":0})), ctx)
        .await
        .expect("repeat wait");
    assert_eq!(output(&repeated)["timed_out"], true);
}

#[tokio::test]
async fn interrupt_reaches_child_while_detached_monitor_waits() {
    let control = CollaborationControl::default();
    let host = Arc::new(TestHost::new(new_session_id()));
    let spawn = SpawnAgentTool::new(vec![role()], 10_000, control.clone());
    let interrupt = InterruptAgentTool::new(10_000, control.clone());
    let wait = WaitAgentTool::new(10_000, control);
    let ctx = context(host);

    spawn
        .invoke(
            &call(
                "spawn_agent",
                json!({"task_name":"scan","message":"inspect","agent_type":"explore"}),
            ),
            ctx.clone(),
        )
        .await
        .expect("spawn");
    interrupt
        .invoke(
            &call("interrupt_agent", json!({"target":"/root/scan"})),
            ctx.clone(),
        )
        .await
        .expect("interrupt");
    let result = wait
        .invoke(&call("wait_agent", json!({"timeout_ms":1000})), ctx)
        .await
        .expect("wait");
    assert_eq!(output(&result)["agents"][0]["status"], "cancelled");
}

#[tokio::test]
async fn ownership_is_session_scoped_and_task_names_are_unique() {
    let control = CollaborationControl::default();
    let first = Arc::new(TestHost::new(new_session_id()));
    let second = Arc::new(TestHost::new(new_session_id()));
    let spawn = SpawnAgentTool::new(vec![role()], 10_000, control.clone());
    let list = ListAgentsTool::new(10_000, control);
    let args = json!({"task_name":"scan","message":"inspect","agent_type":"explore"});

    let first_result = spawn
        .invoke(&call("spawn_agent", args.clone()), context(first.clone()))
        .await
        .expect("first spawn");
    assert!(first_result.ok);
    let duplicate = spawn
        .invoke(&call("spawn_agent", args.clone()), context(first.clone()))
        .await
        .expect("duplicate");
    assert!(!duplicate.ok);
    let second_result = spawn
        .invoke(&call("spawn_agent", args), context(second.clone()))
        .await
        .expect("other session spawn");
    assert!(second_result.ok);

    let first_list = list
        .invoke(&call("list_agents", json!({})), context(first))
        .await
        .expect("list first");
    let second_list = list
        .invoke(&call("list_agents", json!({})), context(second))
        .await
        .expect("list second");
    assert_eq!(output(&first_list)["agents"].as_array().unwrap().len(), 1);
    assert_eq!(output(&second_list)["agents"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn writer_and_worktree_roles_are_rejected_explicitly() {
    let roles = vec![
        SubagentRoleSpec::new("writer", "writes", "write"),
        SubagentRoleSpec::new("worktree", "writes isolated", "write")
            .with_parallel_safe(true)
            .with_isolation(SubagentIsolation::Worktree),
    ];
    let tool = SpawnAgentTool::new(roles, 10_000, CollaborationControl::default());
    let host = Arc::new(TestHost::new(new_session_id()));
    let ctx = context(host);

    let writer = tool
        .invoke(
            &call(
                "spawn_agent",
                json!({"task_name":"write","message":"edit","agent_type":"writer"}),
            ),
            ctx.clone(),
        )
        .await
        .expect("writer result");
    assert!(!writer.ok);
    assert!(
        writer
            .error
            .as_deref()
            .is_some_and(|error| error.contains("not parallel_safe"))
    );

    let worktree = tool
        .invoke(
            &call(
                "spawn_agent",
                json!({"task_name":"tree","message":"edit","agent_type":"worktree"}),
            ),
            ctx,
        )
        .await
        .expect("worktree result");
    assert!(!worktree.ok);
    assert!(
        worktree
            .error
            .as_deref()
            .is_some_and(|error| error.contains("isolation=none"))
    );
}

#[tokio::test]
async fn spawn_rejects_blank_messages_and_reserved_or_invalid_names() {
    let tool = SpawnAgentTool::new(vec![role()], 10_000, CollaborationControl::default());
    let host = Arc::new(TestHost::new(new_session_id()));
    let ctx = context(host);

    for args in [
        json!({"task_name":"scan","message":"  ","agent_type":"explore"}),
        json!({"task_name":"root","message":"inspect","agent_type":"explore"}),
        json!({"task_name":"bad/name","message":"inspect","agent_type":"explore"}),
    ] {
        let result = tool
            .invoke(&call("spawn_agent", args), ctx.clone())
            .await
            .expect("validation result");
        assert!(!result.ok, "invalid spawn must fail: {result:?}");
    }
}

#[test]
fn spawn_spec_advertises_only_eligible_roles_and_keeps_write_safety_floor() {
    let roles = vec![
        role(),
        SubagentRoleSpec::new("writer", "writes", "write"),
        SubagentRoleSpec::new("tree", "tree", "write")
            .with_parallel_safe(true)
            .with_isolation(SubagentIsolation::Worktree),
    ];
    let spec = spawn_spec(&roles, 42);
    assert_eq!(spec.safety, crate::domain::ToolSafety::WritesFiles);
    assert_eq!(spec.metadata["hot"], true);
    assert_eq!(spec.metadata["category"], "proteus_subagent_control");
    assert_eq!(
        spec.input_schema["properties"]["agent_type"]["enum"],
        json!(["explore"])
    );
}
