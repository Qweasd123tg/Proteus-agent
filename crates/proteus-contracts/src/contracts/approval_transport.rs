use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use crate::domain::{ThreadId, ToolCall, ToolSpec, TurnId};

/// Attribution of a control-plane request (approval, user input) to the
/// execution context that asked for it. `label` carries a human-readable
/// source name (e.g. child agent profile) when the requesting thread is not the
/// main turn loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RequestOrigin {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl RequestOrigin {
    pub fn new(thread_id: ThreadId, turn_id: TurnId) -> Self {
        Self {
            thread_id,
            turn_id,
            label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ApprovalRequest {
    pub call: ToolCall,
    pub cwd: PathBuf,
    pub reason: String,
    pub tool_spec: Option<ToolSpec>,
    /// Who is asking: thread/turn plus optional source label. `None` only
    /// for transports/tests constructed outside a runtime turn.
    pub origin: Option<RequestOrigin>,
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
            origin: None,
        }
    }

    pub fn with_origin(mut self, origin: RequestOrigin) -> Self {
        self.origin = Some(origin);
        self
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
            Self::WorkspaceWrite => "workspace_write",
        }
    }

    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "exact_call" => Some(Self::ExactCall),
            "exact_command" => Some(Self::ExactCommand),
            "workspace_write" => Some(Self::WorkspaceWrite),
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
        Self::from_wire(&value)
            .ok_or_else(|| D::Error::custom(format!("unknown approval cache scope {value:?}")))
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
    fn approval_cache_scope_accepts_canonical_wire_names() {
        assert_eq!(
            serde_json::from_str::<ApprovalCacheScope>("\"exact_call\"").unwrap(),
            ApprovalCacheScope::ExactCall
        );
        assert_eq!(
            serde_json::from_str::<ApprovalCacheScope>("\"exact_command\"").unwrap(),
            ApprovalCacheScope::ExactCommand
        );
        assert_eq!(
            serde_json::from_str::<ApprovalCacheScope>("\"workspace_write\"").unwrap(),
            ApprovalCacheScope::WorkspaceWrite
        );
    }

    #[test]
    fn approval_cache_scope_rejects_aliases_and_unknown_names() {
        for value in [
            "exact_shell",
            "workspace_writes",
            "tool_in_cwd",
            "future_scope",
        ] {
            let json = serde_json::to_string(value).unwrap();
            let error = serde_json::from_str::<ApprovalCacheScope>(&json)
                .expect_err("non-canonical cache scope must fail");
            assert!(error.to_string().contains("unknown approval cache scope"));
        }
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
