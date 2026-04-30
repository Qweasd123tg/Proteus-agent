use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::{ToolCall, ToolSpec};

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ApprovalRequest {
    pub call: ToolCall,
    pub cwd: PathBuf,
    pub reason: String,
    pub tool_spec: Option<ToolSpec>,
}

impl ApprovalRequest {
    pub fn new(
        call: ToolCall,
        cwd: PathBuf,
        reason: impl Into<String>,
        tool_spec: Option<ToolSpec>,
    ) -> Self {
        Self {
            call,
            cwd,
            reason: reason.into(),
            tool_spec,
        }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ApprovalResponse {
    pub approved: bool,
    pub note: Option<String>,
    pub cache: ApprovalCacheScope,
}

impl ApprovalResponse {
    pub fn approve() -> Self {
        Self {
            approved: true,
            note: None,
            cache: ApprovalCacheScope::None,
        }
    }

    pub fn deny(note: impl Into<String>) -> Self {
        Self {
            approved: false,
            note: Some(note.into()),
            cache: ApprovalCacheScope::None,
        }
    }

    pub fn new(approved: bool, note: Option<String>, cache: ApprovalCacheScope) -> Self {
        Self {
            approved,
            note,
            cache,
        }
    }

    pub fn with_cache(mut self, cache: ApprovalCacheScope) -> Self {
        self.cache = cache;
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
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
