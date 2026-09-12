use super::AppApprovalId;
use crate::{
    contracts::RequestOrigin,
    domain::{ToolCall, ToolSpec},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Approval request, адресованный клиенту.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppApprovalRequest {
    pub approval_id: AppApprovalId,
    pub call: ToolCall,
    pub cwd: PathBuf,
    pub reason: String,
    pub tool_spec: Option<ToolSpec>,
    pub preview: Option<AppApprovalPreview>,
    /// Кто запросил approval (thread/turn + метка источника, например роль
    /// субагента). `None` — запрос от транспорта без runtime-атрибуции.
    pub origin: Option<RequestOrigin>,
    /// Монотонный порядковый номер в очереди pending approvals текущего
    /// app-server. Клиенты сортируют по нему.
    pub seq: u64,
}

impl AppApprovalRequest {
    pub fn new(
        approval_id: AppApprovalId,
        call: ToolCall,
        cwd: PathBuf,
        reason: String,
        tool_spec: Option<ToolSpec>,
    ) -> Self {
        Self {
            approval_id,
            call,
            cwd,
            reason,
            tool_spec,
            preview: None,
            origin: None,
            seq: 0,
        }
    }

    pub fn with_preview(mut self, preview: Option<AppApprovalPreview>) -> Self {
        self.preview = preview;
        self
    }

    pub fn with_origin(mut self, origin: Option<RequestOrigin>) -> Self {
        self.origin = origin;
        self
    }

    pub fn with_seq(mut self, seq: u64) -> Self {
        self.seq = seq;
        self
    }
}

/// UI-oriented approval preview. It is advisory only: actual execution must
/// still go through the tool's own validation and policy checks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppApprovalPreview {
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub affected_files: Vec<String>,
    pub body: Option<String>,
    pub language: Option<String>,
    pub metadata: Value,
}

impl AppApprovalPreview {
    pub fn new(
        kind: impl Into<String>,
        title: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            title: title.into(),
            summary: summary.into(),
            affected_files: Vec::new(),
            body: None,
            language: None,
            metadata: Value::Null,
        }
    }

    pub fn with_affected_files(mut self, affected_files: Vec<String>) -> Self {
        self.affected_files = affected_files;
        self
    }

    pub fn with_body(mut self, body: impl Into<String>, language: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self.language = Some(language.into());
        self
    }

    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }
}
