use std::{collections::HashSet, path::PathBuf, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{
    contracts::{ApprovalCacheScope, ApprovalRequest, ApprovalResponse, ApprovalTransport},
    domain::{ExecutionId, ThreadId, ToolSafety},
};

/// Session-таймлайн кеш approvals. Agent requests сохраняют прежний
/// thread-scoped ключ, поэтому main-loop cache переживает несколько Turn, а
/// child-thread остаётся изолирован. Detached requests без chat projection
/// разделяются по `ExecutionId`; запросы вообще без origin образуют отдельный
/// unattributed bucket.
#[derive(Clone)]
pub struct CachedApprovalTransport {
    inner: Arc<dyn ApprovalTransport>,
    approved: Arc<Mutex<HashSet<ApprovalCacheKey>>>,
}

impl CachedApprovalTransport {
    pub fn new(inner: Arc<dyn ApprovalTransport>) -> Self {
        Self {
            inner,
            approved: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

#[async_trait]
impl ApprovalTransport for CachedApprovalTransport {
    fn can_request_approval(&self) -> bool {
        self.inner.can_request_approval()
    }

    async fn request_approval(&self, request: ApprovalRequest) -> Result<ApprovalResponse> {
        // A tool may issue state whose lifetime is narrower than the session
        // cache (for example, a turn-scoped permission grant). Its ToolSpec can
        // therefore opt out without coupling core to a concrete tool name.
        if metadata_disables_cache(&request) {
            return self.inner.request_approval(request).await;
        }

        if self.is_cached(&request).await {
            return Ok(ApprovalResponse::approve().with_note("approval reused from session cache"));
        }

        let response = self.inner.request_approval(request.clone()).await?;
        let cache = sanitized_cache_scope(&request, response.cache);
        if response.approved
            && let Some(key) = ApprovalCacheKey::from_request(&request, cache)
        {
            self.approved.lock().await.insert(key);
        }
        Ok(response)
    }
}

impl CachedApprovalTransport {
    async fn is_cached(&self, request: &ApprovalRequest) -> bool {
        let approved = self.approved.lock().await;
        [
            ApprovalCacheScope::ExactCall,
            ApprovalCacheScope::ExactCommand,
            ApprovalCacheScope::WorkspaceWrite,
        ]
        .into_iter()
        .filter_map(|scope| ApprovalCacheKey::from_request(request, scope))
        .any(|key| approved.contains(&key))
    }
}

fn metadata_disables_cache(request: &ApprovalRequest) -> bool {
    request
        .tool_spec
        .as_ref()
        .and_then(|spec| spec.metadata.get("approval"))
        .and_then(|approval| approval.get("cache"))
        .and_then(|cache| cache.get("disabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ApprovalCacheKey {
    origin: ApprovalCacheOrigin,
    tool_name: String,
    cwd: PathBuf,
    args: Option<String>,
}

impl ApprovalCacheKey {
    fn from_request(request: &ApprovalRequest, scope: ApprovalCacheScope) -> Option<Self> {
        let origin = ApprovalCacheOrigin::from_request(request);
        match scope {
            ApprovalCacheScope::None => None,
            ApprovalCacheScope::ExactCall | ApprovalCacheScope::ExactCommand => Some(Self {
                origin,
                tool_name: request.call.name.clone(),
                cwd: request.cwd.clone(),
                args: Some(canonical_json(&request.call.args)),
            }),
            ApprovalCacheScope::WorkspaceWrite => Some(Self {
                origin,
                tool_name: request.call.name.clone(),
                cwd: request.cwd.clone(),
                args: None,
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ApprovalCacheOrigin {
    AgentThread(ThreadId),
    Execution(ExecutionId),
    Unattributed,
}

impl ApprovalCacheOrigin {
    fn from_request(request: &ApprovalRequest) -> Self {
        match request.origin.as_ref() {
            Some(origin) => origin
                .thread_id
                .map_or(Self::Execution(origin.execution_id), Self::AgentThread),
            None => Self::Unattributed,
        }
    }
}

fn sanitized_cache_scope(
    request: &ApprovalRequest,
    requested_scope: ApprovalCacheScope,
) -> ApprovalCacheScope {
    match requested_scope {
        ApprovalCacheScope::WorkspaceWrite if !allows_workspace_write_scope(request) => {
            ApprovalCacheScope::ExactCall
        }
        scope => scope,
    }
}

fn allows_workspace_write_scope(request: &ApprovalRequest) -> bool {
    if request.call.name.eq_ignore_ascii_case("shell") {
        return false;
    }
    request.tool_spec.as_ref().is_some_and(|spec| {
        matches!(spec.safety, ToolSafety::WritesFiles) && metadata_allows_workspace_write(spec)
    })
}

fn metadata_allows_workspace_write(spec: &crate::domain::ToolSpec) -> bool {
    let Some(approval) = spec.metadata.get("approval") else {
        return false;
    };
    if approval
        .get("cache")
        .and_then(|cache| cache.get("workspace_write"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    ["cache", "cache_scopes"].into_iter().any(|field| {
        approval
            .get(field)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|scopes| {
                scopes
                    .iter()
                    .any(|scope| scope.as_str() == Some("workspace_write"))
            })
    })
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Array(values) => {
            let items = values.iter().map(canonical_json).collect::<Vec<_>>();
            format!("[{}]", items.join(","))
        }
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            let items = entries
                .into_iter()
                .map(|(key, value)| {
                    let key = serde_json::to_string(key).expect("json object key serializes");
                    format!("{key}:{}", canonical_json(value))
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", items.join(","))
        }
        _ => serde_json::to_string(value).expect("json value serializes"),
    }
}

#[cfg(test)]
mod tests {
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
        let main_origin =
            RequestOrigin::for_turn(new_execution_id(), new_thread_id(), new_turn_id());
        let child_origin =
            RequestOrigin::for_turn(new_execution_id(), new_thread_id(), new_turn_id())
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
            .request_approval(
                request("a.txt").with_origin(RequestOrigin::for_execution(execution_a)),
            )
            .await
            .unwrap();
        transport
            .request_approval(
                request("a.txt").with_origin(RequestOrigin::for_execution(execution_b)),
            )
            .await
            .unwrap();
        let cached = transport
            .request_approval(
                request("a.txt").with_origin(RequestOrigin::for_execution(execution_a)),
            )
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
}
