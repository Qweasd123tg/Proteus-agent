use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::{ToolCall, ToolSpec};

#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub call: ToolCall,
    pub cwd: PathBuf,
    pub reason: String,
    pub tool_spec: Option<ToolSpec>,
}

#[derive(Debug, Clone)]
pub struct ApprovalResponse {
    pub approved: bool,
    pub note: Option<String>,
    pub cache: ApprovalCacheScope,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ApprovalCacheScope {
    #[default]
    None,
    ExactCall,
}

#[async_trait]
pub trait ApprovalTransport: Send + Sync {
    fn can_request_approval(&self) -> bool;

    async fn request_approval(&self, request: ApprovalRequest) -> Result<ApprovalResponse>;
}
