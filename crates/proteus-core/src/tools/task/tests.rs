use std::{
    fs,
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use super::*;
use crate::contracts::{SubagentIsolation, SubagentToolHost, Tool};
use crate::domain::ToolSafety;

#[derive(Default)]
struct RecordingSubagentHost {
    requests: Mutex<Vec<SubagentRequest>>,
}

struct WritingSubagentHost;

#[async_trait]
impl SubagentToolHost for WritingSubagentHost {
    async fn run_subagent(&self, request: SubagentRequest) -> Result<SubagentResult> {
        fs::write(request.task.cwd.join("child.txt"), "changed\n")?;
        Ok(
            SubagentResult::new("change complete", SubagentStatus::Completed, 1)
                .with_child_thread_id(crate::domain::new_thread_id()),
        )
    }
}

#[async_trait]
impl SubagentToolHost for RecordingSubagentHost {
    async fn run_subagent(&self, request: SubagentRequest) -> Result<SubagentResult> {
        self.requests.lock().unwrap().push(request);
        Ok(
            SubagentResult::new("child summary", SubagentStatus::Completed, 2)
                .with_child_thread_id(crate::domain::new_thread_id()),
        )
    }
}

fn role(name: &str) -> SubagentRoleSpec {
    SubagentRoleSpec::new(name, "Read-only exploration", "Inspect files.")
}

#[test]
fn spec_describes_roles_parallelism_and_resume() {
    let roles = vec![
        role("explore").with_parallel_safe(true),
        role("coder").with_isolation(SubagentIsolation::Worktree),
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

#[tokio::test]
async fn invoke_delegates_through_runtime_bound_host() {
    let host = Arc::new(RecordingSubagentHost::default());
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
    let mut ctx = ToolContext::new(parent_task.cwd.clone());
    ctx.task = Some(parent_task.clone());
    ctx.subagent = Some(host.clone());

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
        vec![role("coder").with_isolation(SubagentIsolation::Worktree)],
        30_000,
    );
    let parent_task = crate::domain::AgentTask::new("fix", repo.path().to_path_buf());
    let call = ToolCall::new(
        "task-wt",
        TASK_TOOL,
        json!({"agent_type": "coder", "prompt": "make a change"}),
    );
    let mut ctx = ToolContext::new(parent_task.cwd.clone());
    ctx.task = Some(parent_task);
    ctx.subagent = Some(Arc::new(WritingSubagentHost));

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
