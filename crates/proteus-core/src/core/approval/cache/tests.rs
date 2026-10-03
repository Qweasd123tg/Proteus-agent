use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serde_json::json;

use super::*;
use crate::domain::{ToolCall, ToolSafety, ToolSpec, new_call_id};

#[derive(Debug)]
struct CountingApprovalTransport {
    calls: Arc<AtomicUsize>,
    cache: ApprovalCacheScope,
}

#[async_trait]
impl ApprovalTransport for CountingApprovalTransport {
    fn can_request_approval(&self) -> bool {
        true
    }

    async fn request_approval(&self, _request: ApprovalRequest) -> Result<ApprovalResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ApprovalResponse::approve().with_cache(self.cache))
    }
}

fn request(path: &str) -> ApprovalRequest {
    ApprovalRequest::new(
        ToolCall::new(
            new_call_id(),
            "write_file",
            json!({ "path": path, "content": "x" }),
        ),
        PathBuf::from("/workspace"),
        "test",
        None,
    )
}

fn request_with_safety(path: &str, tool_name: &str, safety: ToolSafety) -> ApprovalRequest {
    ApprovalRequest::new(
        ToolCall::new(
            new_call_id(),
            tool_name,
            json!({ "path": path, "content": "x" }),
        ),
        PathBuf::from("/workspace"),
        "test",
        Some(ToolSpec::new(tool_name, "test tool", json!({}), safety)),
    )
}

fn request_with_workspace_write_metadata(path: &str, tool_name: &str) -> ApprovalRequest {
    ApprovalRequest::new(
        ToolCall::new(
            new_call_id(),
            tool_name,
            json!({ "path": path, "content": "x" }),
        ),
        PathBuf::from("/workspace"),
        "test",
        Some(
            ToolSpec::new(tool_name, "test tool", json!({}), ToolSafety::WritesFiles)
                .with_metadata(json!({
                    "approval": {
                        "cache_scopes": ["workspace_write"]
                    }
                })),
        ),
    )
}

fn shell_request(command: &str, cwd: &str) -> ApprovalRequest {
    ApprovalRequest::new(
        ToolCall::new(new_call_id(), "shell", json!({ "command": command })),
        PathBuf::from(cwd),
        "test",
        Some(ToolSpec::new(
            "shell",
            "Run command",
            json!({}),
            ToolSafety::RunsCommands,
        )),
    )
}

fn request_permissions() -> ApprovalRequest {
    ApprovalRequest::new(
        ToolCall::new(
            new_call_id(),
            "request_permissions",
            json!({
                "permissions": ["escalated_exec"],
                "justification": "test turn-scoped grant"
            }),
        ),
        PathBuf::from("/workspace"),
        "test",
        Some(
            ToolSpec::new(
                "request_permissions",
                "Request turn-scoped permissions",
                json!({}),
                ToolSafety::RunsCommands,
            )
            .with_metadata(json!({
                "approval": {
                    "cache": { "disabled": true }
                }
            })),
        ),
    )
}

#[tokio::test]
async fn exact_call_cache_reuses_identical_approval() {
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::ExactCall,
    }));

    transport.request_approval(request("a.txt")).await.unwrap();
    let cached = transport.request_approval(request("a.txt")).await.unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(cached.approved);
    assert!(cached.note.unwrap().contains("session cache"));
}

#[tokio::test]
async fn exact_call_cache_does_not_reuse_different_args() {
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::ExactCall,
    }));

    transport.request_approval(request("a.txt")).await.unwrap();
    transport.request_approval(request("b.txt")).await.unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// Кеш скоупится по thread запросившего: approve, выданный субагенту,
/// не действует для main-цикла (и наоборот), а также не делится между
/// разными запусками субагентов.
#[tokio::test]
async fn cache_is_scoped_to_requesting_thread() {
    use crate::contracts::RequestOrigin;
    use crate::domain::{new_execution_id, new_thread_id, new_turn_id};

    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::ExactCall,
    }));
    let main_origin = RequestOrigin::for_turn(new_execution_id(), new_thread_id(), new_turn_id());
    let child_origin = RequestOrigin::for_turn(new_execution_id(), new_thread_id(), new_turn_id())
        .with_label("explore");

    transport
        .request_approval(request("a.txt").with_origin(main_origin.clone()))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Тот же вызов из другого thread-а спрашивает заново.
    transport
        .request_approval(request("a.txt").with_origin(child_origin.clone()))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    // Запрос без origin — собственный bucket, а не подмножество чужого.
    transport.request_approval(request("a.txt")).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    // Повтор внутри своего thread-а переиспользуется.
    let cached = transport
        .request_approval(request("a.txt").with_origin(main_origin))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(cached.approved);
    assert!(cached.note.unwrap().contains("session cache"));
}

#[tokio::test]
async fn agent_cache_keeps_thread_semantics_across_executions() {
    use crate::contracts::RequestOrigin;
    use crate::domain::{new_execution_id, new_thread_id, new_turn_id};

    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::ExactCall,
    }));
    let thread_id = new_thread_id();

    transport
        .request_approval(request("a.txt").with_origin(RequestOrigin::for_turn(
            new_execution_id(),
            thread_id,
            new_turn_id(),
        )))
        .await
        .unwrap();
    let cached = transport
        .request_approval(request("a.txt").with_origin(RequestOrigin::for_turn(
            new_execution_id(),
            thread_id,
            new_turn_id(),
        )))
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(cached.note.unwrap().contains("session cache"));
}

#[tokio::test]
async fn detached_cache_is_isolated_by_execution() {
    use crate::contracts::RequestOrigin;
    use crate::domain::new_execution_id;

    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::ExactCall,
    }));
    let execution_a = new_execution_id();
    let execution_b = new_execution_id();

    transport
        .request_approval(request("a.txt").with_origin(RequestOrigin::for_execution(execution_a)))
        .await
        .unwrap();
    transport
        .request_approval(request("a.txt").with_origin(RequestOrigin::for_execution(execution_b)))
        .await
        .unwrap();
    let cached = transport
        .request_approval(request("a.txt").with_origin(RequestOrigin::for_execution(execution_a)))
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(cached.note.unwrap().contains("session cache"));
}

#[tokio::test]
async fn request_permissions_approval_is_not_reused_across_turns() {
    use crate::contracts::RequestOrigin;
    use crate::domain::{new_execution_id, new_thread_id, new_turn_id};

    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::ExactCall,
    }));
    let thread_id = new_thread_id();

    transport
        .request_approval(request_permissions().with_origin(RequestOrigin::for_turn(
            new_execution_id(),
            thread_id,
            new_turn_id(),
        )))
        .await
        .unwrap();
    transport
        .request_approval(request_permissions().with_origin(RequestOrigin::for_turn(
            new_execution_id(),
            thread_id,
            new_turn_id(),
        )))
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn request_permissions_approval_is_not_reused_within_a_turn() {
    use crate::contracts::RequestOrigin;
    use crate::domain::{new_execution_id, new_thread_id, new_turn_id};

    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::ExactCall,
    }));
    let origin = RequestOrigin::for_turn(new_execution_id(), new_thread_id(), new_turn_id());

    transport
        .request_approval(request_permissions().with_origin(origin.clone()))
        .await
        .unwrap();
    transport
        .request_approval(request_permissions().with_origin(origin))
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn exact_command_cache_reuses_identical_shell_command_in_same_cwd() {
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::ExactCommand,
    }));

    transport
        .request_approval(shell_request("cargo test", "/workspace"))
        .await
        .unwrap();
    let cached = transport
        .request_approval(shell_request("cargo test", "/workspace"))
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(cached.approved);
    assert!(cached.note.unwrap().contains("session cache"));
}

#[tokio::test]
async fn exact_command_cache_does_not_reuse_different_cwd_or_command() {
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::ExactCommand,
    }));

    transport
        .request_approval(shell_request("cargo test", "/workspace"))
        .await
        .unwrap();
    transport
        .request_approval(shell_request("cargo test", "/other-workspace"))
        .await
        .unwrap();
    transport
        .request_approval(shell_request("cargo check", "/workspace"))
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn workspace_write_cache_reuses_opted_in_workspace_write_tools() {
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::WorkspaceWrite,
    }));

    transport
        .request_approval(request_with_workspace_write_metadata("a.txt", "write_file"))
        .await
        .unwrap();
    let cached = transport
        .request_approval(request_with_workspace_write_metadata("b.txt", "write_file"))
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(cached.approved);
    assert!(cached.note.unwrap().contains("session cache"));
}

#[tokio::test]
async fn workspace_write_cache_requires_tool_metadata_opt_in() {
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::WorkspaceWrite,
    }));

    transport
        .request_approval(request_with_safety(
            "a.txt",
            "custom_write",
            ToolSafety::WritesFiles,
        ))
        .await
        .unwrap();
    transport
        .request_approval(request_with_safety(
            "b.txt",
            "custom_write",
            ToolSafety::WritesFiles,
        ))
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn workspace_write_cache_rechecks_current_tool_eligibility() {
    for changed_safety in [
        None,
        Some(ToolSafety::RunsCommands),
        Some(ToolSafety::WritesFiles),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
            calls: calls.clone(),
            cache: ApprovalCacheScope::WorkspaceWrite,
        }));
        transport
            .request_approval(request_with_workspace_write_metadata(
                "a.txt",
                "custom_write",
            ))
            .await
            .unwrap();

        let mut changed = request_with_workspace_write_metadata("b.txt", "custom_write");
        match changed_safety.clone() {
            None => changed.tool_spec = None,
            Some(ToolSafety::WritesFiles) => {
                changed.tool_spec.as_mut().unwrap().metadata = json!({});
            }
            Some(safety) => changed.tool_spec.as_mut().unwrap().safety = safety,
        }
        transport.request_approval(changed.clone()).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2, "{changed_safety:?}");

        // The new exact approval still works; losing broad eligibility does
        // not disable the narrower scope that the user just approved.
        transport.request_approval(changed).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        transport
            .request_approval(request_with_workspace_write_metadata(
                "c.txt",
                "custom_write",
            ))
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn exact_call_cache_canonicalizes_json_object_order() {
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
        calls: calls.clone(),
        cache: ApprovalCacheScope::ExactCall,
    }));

    let mut first = request("a.txt");
    first.call.args = json!({ "path": "a.txt", "content": "x" });
    let mut second = request("a.txt");
    second.call.args = json!({ "content": "x", "path": "a.txt" });
    transport.request_approval(first).await.unwrap();
    transport.request_approval(second).await.unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
