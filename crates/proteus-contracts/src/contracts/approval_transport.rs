use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ApprovalCacheScope {
    #[default]
    None,
    /// Reuse only an identical tool call in the same cwd.
    ExactCall,
    /// Command-shaped exact call. Uses the same cache key as `ExactCall`, but
    /// lets clients present shell/process approvals as "same command".
    ExactCommand,
    /// Legacy broad scope: reuse by tool name and cwd. New clients should use
    /// `WorkspaceWrite` for write tools and `ExactCommand` for shell/process.
    ToolInCwd,
    /// Reuse workspace-scoped write tools by tool name and cwd. Core only
    /// accepts this broad scope when the tool explicitly opts in via metadata.
    WorkspaceWrite,
}

impl ApprovalCacheScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ExactCall => "exact_call",
            Self::ExactCommand => "exact_command",
            Self::ToolInCwd => "tool_in_cwd",
            Self::WorkspaceWrite => "workspace_write",
        }
    }

    pub fn from_wire(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "no_cache" | "once" => Some(Self::None),
            "exact" | "exact_call" | "exact_tool_call" => Some(Self::ExactCall),
            "exact_command" | "exact_shell" | "same_command" | "shell_command" => {
                Some(Self::ExactCommand)
            }
            "tool_in_cwd" | "tool_cwd" | "tool_in_workspace" => Some(Self::ToolInCwd),
            "workspace_write" | "workspace_writes" | "write_in_workspace" => {
                Some(Self::WorkspaceWrite)
            }
            _ => None,
        }
    }
}

impl Serialize for ApprovalCacheScope {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ApprovalCacheScope {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::from_wire(&value).unwrap_or(Self::None))
    }
}

#[async_trait]
pub trait ApprovalTransport: Send + Sync {
    fn can_request_approval(&self) -> bool;

    async fn request_approval(&self, request: ApprovalRequest) -> Result<ApprovalResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_cache_scope_accepts_current_and_legacy_wire_names() {
        assert_eq!(
            serde_json::from_str::<ApprovalCacheScope>("\"exact_call\"").unwrap(),
            ApprovalCacheScope::ExactCall
        );
        assert_eq!(
            serde_json::from_str::<ApprovalCacheScope>("\"exact_shell\"").unwrap(),
            ApprovalCacheScope::ExactCommand
        );
        assert_eq!(
            serde_json::from_str::<ApprovalCacheScope>("\"workspace_writes\"").unwrap(),
            ApprovalCacheScope::WorkspaceWrite
        );
        assert_eq!(
            serde_json::from_str::<ApprovalCacheScope>("\"tool_in_cwd\"").unwrap(),
            ApprovalCacheScope::ToolInCwd
        );
    }

    #[test]
    fn approval_cache_scope_downgrades_unknown_wire_names_to_none() {
        assert_eq!(
            serde_json::from_str::<ApprovalCacheScope>("\"future_scope\"").unwrap(),
            ApprovalCacheScope::None
        );
    }

    #[test]
    fn approval_cache_scope_serializes_canonical_names() {
        assert_eq!(
            serde_json::to_string(&ApprovalCacheScope::ExactCommand).unwrap(),
            "\"exact_command\""
        );
        assert_eq!(
            serde_json::to_string(&ApprovalCacheScope::WorkspaceWrite).unwrap(),
            "\"workspace_write\""
        );
    }
}
