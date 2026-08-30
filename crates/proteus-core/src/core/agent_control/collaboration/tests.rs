use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::control::{FollowupRequest, MAX_OUTSTANDING_COMPLETIONS};
use super::spec::{followup_spec, send_message_spec};
use super::*;
use crate::{
    contracts::{
        AgentControlHandle, AgentControlMessage, AgentControlRequest, AgentControlResult,
        AgentControlToolHost, AgentIsolation, AgentLifecycleStatus, AgentProfile,
        CancellationToken, ExecutionAttribution,
    },
    domain::{AgentTask, ToolCall, new_call_id, new_execution_id, new_session_id, new_thread_id},
};

struct TestHost {
    session_id: SessionId,
    finished: CancellationToken,
    cancelled: CancellationToken,
    requests: Mutex<Vec<AgentControlRequest>>,
    messages: Mutex<Vec<String>>,
}

impl TestHost {
    fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            finished: CancellationToken::new(),
            cancelled: CancellationToken::new(),
            requests: Mutex::new(Vec::new()),
            messages: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl AgentControlToolHost for TestHost {
    fn session_id(&self) -> Option<SessionId> {
        Some(self.session_id)
    }

    async fn run_agent(&self, _request: AgentControlRequest) -> Result<AgentControlResult> {
        unreachable!("collaboration tools use spawn")
    }

    async fn spawn_agent(&self, request: AgentControlRequest) -> Result<AgentControlHandle> {
        self.requests.lock().unwrap().push(request);
        Ok(AgentControlHandle::new(
            new_call_id(),
            "explore",
            new_thread_id(),
        ))
    }

    async fn wait_agent(&self, handle: &AgentControlHandle) -> Result<AgentControlResult> {
        tokio::select! {
            _ = self.cancelled.cancelled() => Ok(AgentControlResult::new(
                "partial",
                AgentLifecycleStatus::Cancelled,
            )
            .with_child_thread_id(handle.child_thread_id)
            .with_metadata(json!({ "resumable": true }))),
            _ = self.finished.cancelled() => Ok(AgentControlResult::new(
                "done",
                AgentLifecycleStatus::Completed,
            )
            .with_child_thread_id(handle.child_thread_id)
            .with_metadata(json!({ "resumable": true }))),
        }
    }

    async fn cancel_agent(&self, _handle: &AgentControlHandle) -> Result<()> {
        self.cancelled.cancel();
        Ok(())
    }

    async fn send_agent(
        &self,
        _handle: &AgentControlHandle,
        message: AgentControlMessage,
    ) -> Result<()> {
        self.messages.lock().unwrap().push(message.content);
        Ok(())
    }
}

fn role() -> AgentProfile {
    AgentProfile::new("explore", "read only").with_parallel_safe(true)
}

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall::new(new_call_id(), name, args)
}

fn context(host: Arc<TestHost>) -> ToolContext {
    let mut ctx = ToolContext::new(
        std::env::current_dir().expect("cwd"),
        ExecutionAttribution::detached(new_execution_id()),
    );
    ctx.task = Some(AgentTask::new("parent", ctx.cwd.clone()));
    ctx.agent_control = Some(host);
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
async fn running_send_and_followup_use_the_child_mailbox_without_spawning() {
    let control = CollaborationControl::default();
    let host = Arc::new(TestHost::new(new_session_id()));
    let spawn = SpawnAgentTool::new(vec![role()], 10_000, control.clone());
    let send = SendMessageTool::new(10_000, control.clone());
    let followup = FollowupTaskTool::new(10_000, control);
    let ctx = context(host.clone());

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
    let sent = send
        .invoke(
            &call(
                "send_message",
                json!({"target":"scan","message":"check tests too"}),
            ),
            ctx.clone(),
        )
        .await
        .expect("send");
    let followed = followup
        .invoke(
            &call(
                "followup_task",
                json!({"target":"/root/scan","message":"summarize risks"}),
            ),
            ctx,
        )
        .await
        .expect("followup");

    assert_eq!(output(&sent)["turn_started"], false);
    assert_eq!(output(&followed)["turn_started"], false);
    assert_eq!(host.requests.lock().unwrap().len(), 1);
    assert_eq!(
        *host.messages.lock().unwrap(),
        vec!["check tests too", "summarize risks"]
    );
}

#[tokio::test]
async fn terminal_followup_resumes_same_task_and_keeps_old_completion_immutable() {
    let control = CollaborationControl::default();
    let host = Arc::new(TestHost::new(new_session_id()));
    let spawn = SpawnAgentTool::new(vec![role()], 10_000, control.clone());
    let send = SendMessageTool::new(10_000, control.clone());
    let followup = FollowupTaskTool::new(10_000, control.clone());
    let wait = WaitAgentTool::new(10_000, control);
    let ctx = context(host.clone());

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
    host.finished.cancel();
    tokio::task::yield_now().await;

    let idle_send = send
        .invoke(
            &call("send_message", json!({"target":"scan","message":"late"})),
            ctx.clone(),
        )
        .await
        .expect("idle send");
    assert!(!idle_send.ok);
    assert!(
        idle_send
            .error
            .as_deref()
            .unwrap()
            .contains("followup_task")
    );

    let resumed = followup
        .invoke(
            &call(
                "followup_task",
                json!({"target":"scan","message":"continue"}),
            ),
            ctx.clone(),
        )
        .await
        .expect("resume");
    assert_eq!(output(&resumed)["turn_started"], true);
    assert_eq!(output(&resumed)["generation"], 2);

    {
        let requests = host.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].metadata["task_id"].is_string());
    }

    let updates = wait
        .invoke(&call("wait_agent", json!({"timeout_ms":1000})), ctx)
        .await
        .expect("wait updates");
    let updates = output(&updates);
    let first = updates["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["generation"] == 1)
        .expect("generation one completion retained");
    assert_eq!(first["status"], "completed");
}

#[test]
fn followup_reservation_is_atomic_and_stale_completion_cannot_overwrite_it() {
    let control = CollaborationControl::default();
    let session_id = new_session_id();
    let host = Arc::new(TestHost::new(session_id));
    let reservation = control
        .reserve(session_id, "scan", "explore")
        .expect("reserve");
    let first_handle = AgentControlHandle::new(new_call_id(), "explore", new_thread_id());
    control
        .attach(
            session_id,
            &reservation.path,
            reservation.generation,
            first_handle.clone(),
            host.clone(),
        )
        .expect("attach");
    control.complete(
        session_id,
        &reservation.path,
        reservation.generation,
        Ok(
            AgentControlResult::new("done", AgentLifecycleStatus::Completed)
                .with_child_thread_id(first_handle.child_thread_id)
                .with_metadata(json!({ "resumable": true })),
        ),
    );

    let idle = match control
        .begin_followup(session_id, "scan")
        .expect("begin followup")
    {
        FollowupRequest::Idle(idle) => idle,
        FollowupRequest::Running(_) => panic!("terminal record must reserve a new generation"),
    };
    assert!(control.begin_followup(session_id, "scan").is_err());

    let second_handle =
        AgentControlHandle::new(new_call_id(), "explore", first_handle.child_thread_id);
    control
        .attach_followup(session_id, &idle, second_handle, host)
        .expect("attach followup");
    control.complete(
        session_id,
        &idle.path,
        reservation.generation,
        Ok(AgentControlResult::new(
            "stale",
            AgentLifecycleStatus::Cancelled,
        )),
    );

    let view = control
        .list(session_id, Some("scan"))
        .expect("list")
        .pop()
        .expect("agent");
    assert_eq!(view.generation, 2);
    assert_eq!(view.status, AgentLifecycleStatus::Running);
}

#[test]
fn non_resumable_completion_does_not_advertise_followup_target() {
    let control = CollaborationControl::default();
    let session_id = new_session_id();
    let host = Arc::new(TestHost::new(session_id));
    let reservation = control
        .reserve(session_id, "scan", "explore")
        .expect("reserve");
    let child_thread_id = new_thread_id();
    control
        .attach(
            session_id,
            &reservation.path,
            reservation.generation,
            AgentControlHandle::new(new_call_id(), "explore", child_thread_id),
            host,
        )
        .expect("attach");
    control.complete(
        session_id,
        &reservation.path,
        reservation.generation,
        Ok(
            AgentControlResult::new("done", AgentLifecycleStatus::Completed)
                .with_child_thread_id(child_thread_id)
                .with_metadata(json!({ "resumable": false })),
        ),
    );

    let view = control
        .list(session_id, Some("scan"))
        .expect("list")
        .pop()
        .expect("agent");
    assert_eq!(view.child_thread_id, None);
    let error = match control.begin_followup(session_id, "scan") {
        Ok(_) => panic!("non-resumable result must reject follow-up"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("no resumable task id"));
}

#[test]
fn followup_generations_stop_before_completion_queue_can_grow_unbounded() {
    let control = CollaborationControl::default();
    let session_id = new_session_id();
    let host = Arc::new(TestHost::new(session_id));
    let reservation = control
        .reserve(session_id, "scan", "explore")
        .expect("reserve");
    let child_thread_id = new_thread_id();
    let first = AgentControlHandle::new(new_call_id(), "explore", child_thread_id);
    control
        .attach(
            session_id,
            &reservation.path,
            reservation.generation,
            first,
            host.clone(),
        )
        .expect("attach");
    control.complete(
        session_id,
        &reservation.path,
        1,
        Ok(
            AgentControlResult::new("done", AgentLifecycleStatus::Completed)
                .with_child_thread_id(child_thread_id)
                .with_metadata(json!({ "resumable": true })),
        ),
    );

    for generation in 2..=MAX_OUTSTANDING_COMPLETIONS as u64 {
        let idle = match control
            .begin_followup(session_id, "scan")
            .expect("capacity remains")
        {
            FollowupRequest::Idle(idle) => idle,
            FollowupRequest::Running(_) => panic!("record is terminal"),
        };
        assert_eq!(idle.generation, generation);
        control
            .attach_followup(
                session_id,
                &idle,
                AgentControlHandle::new(new_call_id(), "explore", child_thread_id),
                host.clone(),
            )
            .expect("attach generation");
        control.complete(
            session_id,
            &idle.path,
            generation,
            Ok(
                AgentControlResult::new("done", AgentLifecycleStatus::Completed)
                    .with_child_thread_id(child_thread_id)
                    .with_metadata(json!({ "resumable": true })),
            ),
        );
    }

    let error = match control.begin_followup(session_id, "scan") {
        Ok(_) => panic!("queue cap must reject another generation"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("wait_agent"));
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
        AgentProfile::new("writer", "writes"),
        AgentProfile::new("worktree", "writes isolated")
            .with_parallel_safe(true)
            .with_isolation(AgentIsolation::Worktree),
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
        AgentProfile::new("writer", "writes"),
        AgentProfile::new("tree", "tree")
            .with_parallel_safe(true)
            .with_isolation(AgentIsolation::Worktree),
    ];
    let spec = spawn_spec(&roles, 42);
    assert_eq!(spec.safety, crate::domain::ToolSafety::WritesFiles);
    assert_eq!(spec.metadata["hot"], true);
    assert_eq!(spec.metadata["category"], "proteus_agent_control");
    assert_eq!(
        spec.input_schema["properties"]["agent_type"]["enum"],
        json!(["explore"])
    );
}

#[test]
fn messaging_specs_keep_write_safety_floor() {
    for spec in [send_message_spec(42), followup_spec(42)] {
        assert_eq!(spec.safety, crate::domain::ToolSafety::WritesFiles);
        assert_eq!(spec.metadata["category"], "proteus_agent_control");
        assert_eq!(spec.input_schema["required"], json!(["target", "message"]));
    }
}

#[test]
fn collaboration_surface_registers_lifecycle_and_messaging_tools() {
    let mut tools = ToolRegistry::new();
    register_collaboration_tools(&mut tools, vec![role()], 42).expect("collaboration tools");
    for name in COLLABORATION_TOOL_NAMES {
        assert!(tools.spec(name).is_ok(), "missing {name}");
    }
}
