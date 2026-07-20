use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ToolCallInfo;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApprovalCacheScope {
    #[default]
    None,
    ExactCall,
    ExactCommand,
    WorkspaceWrite,
}

impl ApprovalCacheScope {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "Один раз",
            Self::ExactCall => "Точно",
            Self::ExactCommand => "Команда",
            Self::WorkspaceWrite => "Workspace",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ApprovalRequestInfo {
    pub(crate) approval_id: String,
    pub(crate) call: ToolCallInfo,
    pub(crate) cwd: String,
    pub(crate) reason: String,
    pub(crate) tool_spec: Option<Value>,
    pub(crate) preview: Option<ApprovalPreviewInfo>,
    /// Атрибуция запроса: thread/turn + метка источника (роль субагента).
    pub(crate) origin: Option<RequestOriginInfo>,
    /// Порядковый номер в очереди approvals.
    pub(crate) seq: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct RequestOriginInfo {
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ApprovalPreviewInfo {
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) affected_files: Vec<String>,
    pub(crate) body: Option<String>,
    pub(crate) language: Option<String>,
    pub(crate) metadata: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct UserInputOption {
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) preview: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct UserInputQuestion {
    pub(crate) id: String,
    pub(crate) header: String,
    pub(crate) question: String,
    pub(crate) is_other: bool,
    pub(crate) is_secret: bool,
    pub(crate) multi_select: bool,
    pub(crate) options: Vec<UserInputOption>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct UserInputRequestInfo {
    pub(crate) request_id: String,
    pub(crate) cwd: String,
    pub(crate) title: Option<String>,
    pub(crate) questions: Vec<UserInputQuestion>,
    /// Атрибуция запроса: thread/turn + метка источника (роль субагента).
    pub(crate) origin: Option<RequestOriginInfo>,
    /// Порядковый номер в очереди user inputs.
    pub(crate) seq: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct PendingControlPlaneInfo {
    pub(crate) approvals: Vec<ApprovalRequestInfo>,
    pub(crate) user_inputs: Vec<UserInputRequestInfo>,
    pub(crate) queued_user_messages: Vec<QueuedPromptInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct QueuedPromptInfo {
    pub(crate) message_id: String,
    pub(crate) text: String,
}
