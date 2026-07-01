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
    ToolInCwd,
    WorkspaceWrite,
}

impl ApprovalCacheScope {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "Один раз",
            Self::ExactCall => "Точно",
            Self::ExactCommand => "Команда",
            Self::ToolInCwd => "Tool/CWD",
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
    #[serde(default)]
    pub(crate) preview: Option<ApprovalPreviewInfo>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ApprovalPreviewInfo {
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) affected_files: Vec<String>,
    #[serde(default)]
    pub(crate) body: Option<String>,
    #[serde(default)]
    pub(crate) language: Option<String>,
    #[serde(default)]
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
    #[serde(default)]
    pub(crate) is_other: bool,
    #[serde(default)]
    pub(crate) is_secret: bool,
    #[serde(default, alias = "multiSelect")]
    pub(crate) multi_select: bool,
    #[serde(default)]
    pub(crate) options: Vec<UserInputOption>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct UserInputRequestInfo {
    pub(crate) request_id: String,
    pub(crate) cwd: String,
    pub(crate) title: Option<String>,
    pub(crate) questions: Vec<UserInputQuestion>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct PendingControlPlaneInfo {
    #[serde(default)]
    pub(crate) approvals: Vec<ApprovalRequestInfo>,
    #[serde(default)]
    pub(crate) user_inputs: Vec<UserInputRequestInfo>,
}
