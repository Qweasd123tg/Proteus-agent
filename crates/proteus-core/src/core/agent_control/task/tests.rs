use std::{
    fs,
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use super::*;
use crate::contracts::{
    AgentControlToolHost, AgentIsolation, AgentLifecycleStatus, ExecutionAttribution, Tool,
};
use crate::domain::{ToolSafety, new_execution_id};

fn test_tool_attribution() -> ExecutionAttribution {
    ExecutionAttribution::detached(new_execution_id())
}

#[derive(Default)]
struct RecordingAgentHost {
    requests: Mutex<Vec<AgentControlRequest>>,
}

struct WritingAgentHost;

struct SessionWritingAgentHost {
    session_id: crate::domain::SessionId,
    child_thread_id: crate::domain::ThreadId,
    resumable: Mutex<bool>,
    calls: Mutex<usize>,
}

impl SessionWritingAgentHost {
    fn new(session_id: crate::domain::SessionId, child_thread_id: crate::domain::ThreadId) -> Self {
        Self {
            session_id,
            child_thread_id,
            resumable: Mutex::new(true),
            calls: Mutex::new(0),
        }
    }
}

#[async_trait]
impl AgentControlToolHost for WritingAgentHost {
    async fn run_agent(&self, request: AgentControlRequest) -> Result<AgentControlResult> {
        fs::write(request.task.cwd.join("child.txt"), "changed\n")?;
        Ok(
            AgentControlResult::new("change complete", AgentLifecycleStatus::Completed)
                .with_child_thread_id(crate::domain::new_thread_id()),
        )
    }
}

#[async_trait]
impl AgentControlToolHost for SessionWritingAgentHost {
    fn session_id(&self) -> Option<crate::domain::SessionId> {
        Some(self.session_id)
    }

    async fn run_agent(&self, request: AgentControlRequest) -> Result<AgentControlResult> {
        *self.calls.lock().unwrap() += 1;
        fs::write(request.task.cwd.join("child.txt"), "changed\n")?;
        Ok(
            AgentControlResult::new("change complete", AgentLifecycleStatus::Completed)
                .with_child_thread_id(self.child_thread_id)
                .with_metadata(json!({ "resumable": *self.resumable.lock().unwrap() })),
        )
    }
}

#[async_trait]
impl AgentControlToolHost for RecordingAgentHost {
    async fn run_agent(&self, request: AgentControlRequest) -> Result<AgentControlResult> {
        self.requests.lock().unwrap().push(request);
        Ok(
            AgentControlResult::new("child summary", AgentLifecycleStatus::Completed)
                .with_child_thread_id(crate::domain::new_thread_id()),
        )
    }
}

fn role(name: &str) -> AgentProfile {
    AgentProfile::new(name, "Read-only exploration")
}

#[test]
fn spec_describes_roles_parallelism_and_resume() {
    let roles = vec![
        role("explore").with_parallel_safe(true),
        role("coder").with_isolation(AgentIsolation::Worktree),
    ];
    let spec = task_tool_spec(&roles, 42);

    assert_eq!(spec.name, TASK_TOOL);
    assert_eq!(spec.safety, ToolSafety::WritesFiles);
    assert_eq!(spec.timeout_ms, Some(42));
    assert_eq!(
        spec.input_schema["required"],
        json!(["prompt", "agent_type"])
    );
    let role_help = spec.input_schema["properties"]["agent_type"]["description"]
        .as_str()
        .unwrap();
    assert!(role_help.contains("explore (parallel-safe)"));
    assert!(role_help.contains("coder (worktree-isolated)"));
    assert!(spec.description.contains("NOTHING is merged automatically"));
}

#[test]
fn parallel_gate_requires_every_role_to_be_eligible() {
    let roles = vec![role("explore").with_parallel_safe(true), role("writer")];
    let calls = |second: &str| {
        vec![
            ToolCall::new(
                "1",
                TASK_TOOL,
                json!({"agent_type": "explore", "prompt": "a"}),
            ),
            ToolCall::new("2", TASK_TOOL, json!({"agent_type": second, "prompt": "b"})),
        ]
    };

    assert!(calls_are_parallel_eligible(&calls("explore"), &roles));
    assert!(!calls_are_parallel_eligible(&calls("writer"), &roles));
    assert!(!calls_are_parallel_eligible(&calls("missing"), &roles));
}

#[test]
fn parser_keeps_resume_metadata_and_rejects_wrong_optional_type() {
    let task = crate::domain::AgentTask::new("parent", "/tmp".into());
    let call = ToolCall::new(
        "1",
        TASK_TOOL,
        json!({
            "agent_type": "explore",
            "prompt": "inspect",
            "description": "map code",
            "task_id": "thread-1"
        }),
    );
    let parsed = parse_task_call(&call, task.clone()).unwrap();
    assert_eq!(parsed.task.text, task.text);
    assert_eq!(parsed.description.as_deref(), Some("map code"));
    assert_eq!(parsed.metadata["task_id"], "thread-1");

    let bad = ToolCall::new(
        "2",
        TASK_TOOL,
        json!({"agent_type": "explore", "prompt": "inspect", "task_id": 7}),
    );
    assert!(
        parse_task_call(&bad, task)
            .unwrap_err()
            .contains("must be a string")
    );
}

#[test]
fn task_id_is_published_only_for_explicitly_resumable_results() {
    let thread_id = crate::domain::new_thread_id();
    let non_resumable = AgentControlResult::new("done", AgentLifecycleStatus::Completed)
        .with_child_thread_id(thread_id)
        .with_metadata(json!({ "resumable": false }));
    assert_eq!(child_task_id(&non_resumable), None);

    let resumable = AgentControlResult::new("done", AgentLifecycleStatus::Completed)
        .with_child_thread_id(thread_id)
        .with_metadata(json!({ "resumable": true }));
    assert_eq!(child_task_id(&resumable), Some(thread_id.to_string()));
}

#[tokio::test]
async fn invoke_delegates_through_runtime_bound_host() {
    let host = Arc::new(RecordingAgentHost::default());
    let tool = TaskTool::new(vec![role("explore")], 1_000);
    let parent_task = crate::domain::AgentTask::new("parent task", "/tmp".into());
    let call = ToolCall::new(
        "task-1",
        TASK_TOOL,
        json!({
            "agent_type": "explore",
            "prompt": "inspect the code",
            "description": "inspect code"
        }),
    );
    let mut ctx = ToolContext::new(parent_task.cwd.clone(), test_tool_attribution());
    ctx.task = Some(parent_task.clone());
    ctx.agent_control = Some(host.clone());

    let result = tool.invoke(&call, ctx).await.unwrap();

    assert!(result.ok);
    assert!(result.output.starts_with("child summary"));
    assert_eq!(result.metadata["tool"], TASK_TOOL);
    let requests = host.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].task.text, parent_task.text);
    assert_eq!(requests[0].prompt, "inspect the code");
}

#[tokio::test]
async fn worktree_role_changes_only_isolated_checkout_after_approval_path_invokes_tool() {
    let repo = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test"],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["add", "."])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["commit", "-q", "-m", "base"])
            .status()
            .unwrap()
            .success()
    );

    let tool = TaskTool::new(
        vec![role("coder").with_isolation(AgentIsolation::Worktree)],
        30_000,
    );
    let parent_task = crate::domain::AgentTask::new("fix", repo.path().to_path_buf());
    let call = ToolCall::new(
        "task-wt",
        TASK_TOOL,
        json!({"agent_type": "coder", "prompt": "make a change"}),
    );
    let mut ctx = ToolContext::new(parent_task.cwd.clone(), test_tool_attribution());
    ctx.task = Some(parent_task);
    ctx.agent_control = Some(Arc::new(WritingAgentHost));

    let result = tool.invoke(&call, ctx).await.unwrap();

    assert!(result.ok, "{:?}", result.error);
    assert!(!repo.path().join("child.txt").exists());
    assert!(result.output.contains("branch: proteus/coder-task-wt"));
    assert!(
        repo.path()
            .join(".proteus/worktrees/coder-task-wt/child.txt")
            .exists()
    );
}

#[tokio::test]
async fn worktree_resume_mapping_is_session_owned_and_drops_non_resumable_edge() {
    let repo = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test"],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["add", "."])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["commit", "-q", "-m", "base"])
            .status()
            .unwrap()
            .success()
    );

    let tool = TaskTool::new(
        vec![role("coder").with_isolation(AgentIsolation::Worktree)],
        30_000,
    );
    let parent_task = crate::domain::AgentTask::new("fix", repo.path().to_path_buf());
    let child_thread_id = crate::domain::new_thread_id();
    let owner = Arc::new(SessionWritingAgentHost::new(
        crate::domain::new_session_id(),
        child_thread_id,
    ));
    let fresh = ToolCall::new(
        "task-owned-wt",
        TASK_TOOL,
        json!({"agent_type": "coder", "prompt": "make a change"}),
    );
    let mut owner_ctx = ToolContext::new(parent_task.cwd.clone(), test_tool_attribution());
    owner_ctx.task = Some(parent_task.clone());
    owner_ctx.agent_control = Some(owner.clone());

    let first = tool.invoke(&fresh, owner_ctx.clone()).await.unwrap();
    assert!(first.ok, "{:?}", first.error);
    assert!(first.output.contains(&child_thread_id.to_string()));

    let foreign = Arc::new(SessionWritingAgentHost::new(
        crate::domain::new_session_id(),
        child_thread_id,
    ));
    let resume = ToolCall::new(
        "task-foreign-wt",
        TASK_TOOL,
        json!({
            "agent_type": "coder",
            "prompt": "continue",
            "task_id": child_thread_id.to_string()
        }),
    );
    let mut foreign_ctx = ToolContext::new(parent_task.cwd.clone(), test_tool_attribution());
    foreign_ctx.task = Some(parent_task.clone());
    foreign_ctx.agent_control = Some(foreign.clone());
    let rejected = tool.invoke(&resume, foreign_ctx).await.unwrap();
    assert!(!rejected.ok);
    assert!(
        rejected
            .error
            .as_deref()
            .is_some_and(|error| error.contains("another session"))
    );
    assert_eq!(*foreign.calls.lock().unwrap(), 0);

    *owner.resumable.lock().unwrap() = false;
    let resumed = tool.invoke(&resume, owner_ctx).await.unwrap();
    assert!(resumed.ok, "{:?}", resumed.error);
    assert!(!resumed.output.contains("[task_id:"));
    assert_eq!(*owner.calls.lock().unwrap(), 2);
}
