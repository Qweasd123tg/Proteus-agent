use std::{collections::BTreeMap, path::PathBuf};

use crate::contracts::{ApprovalCacheScope, UserInputResponse};
use serde::Deserialize;

use crate::domain::PermissionMode;

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendRequest {
    pub id: Option<String>,
    pub text: String,
    #[serde(default)]
    pub options: crate::domain::RunOptions,
    pub session_dir: PathBuf,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRequest {
    pub id: Option<String>,
    pub approval_id: String,
    pub approved: bool,
    pub note: Option<String>,
    #[serde(default)]
    pub cache: ApprovalCacheScope,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserInputRequest {
    pub id: Option<String>,
    pub request_id: String,
    pub response: UserInputResponse,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelRequest {
    pub id: Option<String>,
    pub target_id: String,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPermissionModeRequest {
    pub id: Option<String>,
    pub mode: PermissionMode,
    pub session_dir: PathBuf,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetModelRequest {
    pub id: Option<String>,
    pub model: String,
    pub session_dir: PathBuf,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetReasoningEffortRequest {
    pub id: Option<String>,
    pub effort: Option<String>,
    pub session_dir: PathBuf,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetReasoningEnabledRequest {
    pub id: Option<String>,
    pub enabled: bool,
    pub session_dir: PathBuf,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetConfigBuilderRequest {
    #[serde(default)]
    pub modules: BTreeMap<String, String>,
    #[serde(default)]
    pub module_config: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    /// `None` — не трогать `tools.enabled`; `Some` — заменить список целиком.
    #[serde(default)]
    pub tools_enabled: Option<Vec<String>>,
    /// `None` — не трогать `active_provider`.
    #[serde(default)]
    pub active_provider: Option<String>,
    /// `None` — не трогать `[permissions] mode`.
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeSessionRequest {
    pub id: Option<String>,
    pub session_dir: PathBuf,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewSessionRequest {
    pub id: Option<String>,
    pub source_session_dir: Option<PathBuf>,
}

#[derive(Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteSessionRequest {
    pub id: Option<String>,
    pub session_dir: PathBuf,
}

use crate::domain::MessageId;
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditQueuedMessageRequest {
    pub id: Option<String>,
    pub session_dir: PathBuf,
    pub message_id: MessageId,
    pub text: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteQueuedMessageRequest {
    pub id: Option<String>,
    pub session_dir: PathBuf,
    pub message_id: MessageId,
}
