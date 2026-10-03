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
            ApprovalCacheScope::WorkspaceWrite if allows_workspace_write_scope(request) => {
                Some(Self {
                    origin,
                    tool_name: request.call.name.clone(),
                    cwd: request.cwd.clone(),
                    args: None,
                })
            }
            ApprovalCacheScope::WorkspaceWrite => None,
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
mod tests;
