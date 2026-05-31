use std::{collections::HashSet, path::PathBuf, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{
    contracts::{ApprovalCacheScope, ApprovalRequest, ApprovalResponse, ApprovalTransport},
    domain::ToolSafety,
};

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
        [ApprovalCacheScope::ExactCall, ApprovalCacheScope::ToolInCwd]
            .into_iter()
            .filter_map(|scope| ApprovalCacheKey::from_request(request, scope))
            .any(|key| approved.contains(&key))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ApprovalCacheKey {
    tool_name: String,
    cwd: PathBuf,
    args: Option<String>,
}

impl ApprovalCacheKey {
    fn from_request(request: &ApprovalRequest, scope: ApprovalCacheScope) -> Option<Self> {
        match scope {
            ApprovalCacheScope::None => None,
            ApprovalCacheScope::ExactCall => Some(Self {
                tool_name: request.call.name.clone(),
                cwd: request.cwd.clone(),
                args: Some(canonical_json(&request.call.args)),
            }),
            ApprovalCacheScope::ToolInCwd => Some(Self {
                tool_name: request.call.name.clone(),
                cwd: request.cwd.clone(),
                args: None,
            }),
            _ => None,
        }
    }
}

fn sanitized_cache_scope(
    request: &ApprovalRequest,
    requested_scope: ApprovalCacheScope,
) -> ApprovalCacheScope {
    match requested_scope {
        ApprovalCacheScope::ToolInCwd if !allows_tool_in_cwd_scope(request) => {
            ApprovalCacheScope::ExactCall
        }
        scope => scope,
    }
}

fn allows_tool_in_cwd_scope(request: &ApprovalRequest) -> bool {
    if request.call.name.eq_ignore_ascii_case("shell") {
        return false;
    }
    request
        .tool_spec
        .as_ref()
        .is_none_or(|spec| matches!(spec.safety, ToolSafety::ReadOnly | ToolSafety::WritesFiles))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Array(values) => {
            let items = values.iter().map(canonical_json).collect::<Vec<_>>();
            format!("[{}]", items.join(","))
        }
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
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

    #[tokio::test]
    async fn tool_in_cwd_cache_reuses_different_args_for_same_tool_and_cwd() {
        let calls = Arc::new(AtomicUsize::new(0));
        let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
            calls: calls.clone(),
            cache: ApprovalCacheScope::ToolInCwd,
        }));

        transport.request_approval(request("a.txt")).await.unwrap();
        let cached = transport.request_approval(request("b.txt")).await.unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(cached.approved);
        assert!(cached.note.unwrap().contains("session cache"));
    }

    #[tokio::test]
    async fn tool_in_cwd_cache_does_not_reuse_different_tool() {
        let calls = Arc::new(AtomicUsize::new(0));
        let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
            calls: calls.clone(),
            cache: ApprovalCacheScope::ToolInCwd,
        }));

        transport.request_approval(request("a.txt")).await.unwrap();
        let mut second = request("b.txt");
        second.call.name = "shell".to_owned();
        transport.request_approval(second).await.unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn tool_in_cwd_cache_does_not_reuse_different_cwd() {
        let calls = Arc::new(AtomicUsize::new(0));
        let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
            calls: calls.clone(),
            cache: ApprovalCacheScope::ToolInCwd,
        }));

        transport.request_approval(request("a.txt")).await.unwrap();
        let mut second = request("b.txt");
        second.cwd = PathBuf::from("/other-workspace");
        transport.request_approval(second).await.unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn tool_in_cwd_cache_is_sanitized_for_shell_tool() {
        let calls = Arc::new(AtomicUsize::new(0));
        let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
            calls: calls.clone(),
            cache: ApprovalCacheScope::ToolInCwd,
        }));

        transport
            .request_approval(request_with_safety(
                "a.txt",
                "shell",
                ToolSafety::RunsCommands,
            ))
            .await
            .unwrap();
        transport
            .request_approval(request_with_safety(
                "b.txt",
                "shell",
                ToolSafety::RunsCommands,
            ))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn tool_in_cwd_cache_is_sanitized_for_command_network_and_dangerous_tools() {
        for safety in [
            ToolSafety::RunsCommands,
            ToolSafety::Network,
            ToolSafety::Dangerous,
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
                calls: calls.clone(),
                cache: ApprovalCacheScope::ToolInCwd,
            }));

            transport
                .request_approval(request_with_safety("a.txt", "custom_tool", safety.clone()))
                .await
                .unwrap();
            transport
                .request_approval(request_with_safety("b.txt", "custom_tool", safety))
                .await
                .unwrap();

            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }
    }

    #[tokio::test]
    async fn tool_in_cwd_cache_still_reuses_write_file_tools() {
        let calls = Arc::new(AtomicUsize::new(0));
        let transport = CachedApprovalTransport::new(Arc::new(CountingApprovalTransport {
            calls: calls.clone(),
            cache: ApprovalCacheScope::ToolInCwd,
        }));

        transport
            .request_approval(request_with_safety(
                "a.txt",
                "write_file",
                ToolSafety::WritesFiles,
            ))
            .await
            .unwrap();
        transport
            .request_approval(request_with_safety(
                "b.txt",
                "write_file",
                ToolSafety::WritesFiles,
            ))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
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
