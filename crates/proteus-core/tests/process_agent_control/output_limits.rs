use std::{path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use proteus_contracts::contracts::{
    ApprovalRequest, ApprovalResponse, ApprovalTransport, UserInputRequest, UserInputResponse,
    UserInputTransport,
};
use tokio::sync::Notify;

use super::*;

struct DelayedInteraction {
    waiting: PathBuf,
    release: Notify,
}

impl DelayedInteraction {
    async fn wait(&self) {
        std::fs::write(&self.waiting, "waiting").expect("mark pending interaction");
        self.release.notified().await;
    }
}

#[async_trait]
impl ApprovalTransport for DelayedInteraction {
    fn can_request_approval(&self) -> bool {
        true
    }

    async fn request_approval(
        &self,
        _request: ApprovalRequest,
    ) -> anyhow::Result<ApprovalResponse> {
        self.wait().await;
        Ok(ApprovalResponse::approve())
    }
}

#[async_trait]
impl UserInputTransport for DelayedInteraction {
    fn can_request_user_input(&self) -> bool {
        true
    }

    async fn request_user_input(
        &self,
        _request: UserInputRequest,
    ) -> anyhow::Result<UserInputResponse> {
        self.wait().await;
        Ok(UserInputResponse::empty())
    }
}

fn runner(root: &std::path::Path) -> Arc<dyn AgentControl> {
    use std::os::unix::fs::PermissionsExt;
    let binary = root.join("peer.py");
    std::fs::write(&binary, include_str!("../fixtures/agent_output_peer.py")).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    runner_from_json(json!({
        "binary": binary,
        "max_parallel": 2,
        "roles": [{
            "name": "helper", "description": "Output limit fixture",
            "config": "unused", "parallel_safe": true, "max_processes": 2
        }]
    }))
}

async fn wait_for(path: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("peer reached barrier");
}

#[tokio::test]
async fn output_failure_interrupts_pending_interaction_and_preserves_sibling() {
    for (mode, expected) in [
        ("approval-overflow", "256 buffered outputs"),
        ("input-overflow", "256 buffered outputs"),
        ("bytes-overflow", "33554432 buffered bytes"),
        ("frame-overflow", "frame exceeded 8388608 bytes"),
        ("malformed-output", "invalid subagent stdout JSONL output"),
        ("stdout-closed", "subagent child stdout closed"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let runner = runner(root.path());
        let mut ctx = test_runtime_context();
        let interaction = Arc::new(DelayedInteraction {
            waiting: root.path().join("waiting"),
            release: Notify::new(),
        });
        ctx.execution.approval = interaction.clone();
        ctx.user_input = interaction;
        let task = AgentTask::new("output regression", root.path().to_path_buf());
        let flooded = runner
            .spawn(
                AgentControlRequest::new("helper", mode, task.clone()),
                ctx.clone(),
            )
            .await
            .unwrap();
        wait_for(&root.path().join("waiting")).await;
        wait_for(&root.path().join("flood-started")).await;
        let sibling = runner
            .spawn(AgentControlRequest::new("helper", "healthy", task), ctx)
            .await
            .unwrap();
        let error = tokio::time::timeout(Duration::from_secs(5), runner.wait(&flooded))
            .await
            .expect("overflow must interrupt the pending interaction")
            .expect_err("overflow must fail the peer");
        assert!(format!("{error:#}").contains(expected), "{mode}: {error:#}");
        assert!(!root.path().join("answered").exists());
        let result = tokio::time::timeout(Duration::from_secs(5), runner.wait(&sibling))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.status, AgentLifecycleStatus::Completed);
        assert_eq!(result.summary, "healthy peer");
    }
}

#[tokio::test]
async fn queued_output_preserves_approval_result_and_targeted_cancel() {
    for cancel in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let runner = runner(root.path());
        let mut ctx = test_runtime_context();
        let interaction = Arc::new(DelayedInteraction {
            waiting: root.path().join("waiting"),
            release: Notify::new(),
        });
        ctx.execution.approval = interaction.clone();
        let handle = runner
            .spawn(
                AgentControlRequest::new(
                    "helper",
                    "normal",
                    AgentTask::new("normal", root.path().to_path_buf()),
                ),
                ctx,
            )
            .await
            .unwrap();
        wait_for(&root.path().join("produced")).await;
        if cancel {
            runner.cancel(&handle).await.unwrap();
        } else {
            interaction.release.notify_one();
        }
        let result = tokio::time::timeout(Duration::from_secs(5), runner.wait(&handle))
            .await
            .expect("control response must not be blocked by queued output")
            .unwrap();
        if cancel {
            assert_eq!(result.status, AgentLifecycleStatus::Cancelled);
            assert_eq!(
                result.summary,
                (0..32).map(|i| format!("{i:02};")).collect::<String>()
            );
            assert!(root.path().join("cancelled").exists());
            assert!(!root.path().join("answered").exists());
        } else {
            assert_eq!(result.status, AgentLifecycleStatus::Completed);
            assert_eq!(result.summary, "approved result");
        }
    }
}
