use std::{collections::BTreeMap, path::PathBuf};

use proteus_contracts::contracts::{ApprovalCacheScope, UserInputResponse};
use serde::Deserialize;

use crate::domain::PermissionMode;

#[derive(Debug, Deserialize)]
pub(super) struct SendRequest {
    pub(super) id: Option<String>,
    pub(super) text: String,
    #[serde(default)]
    pub(super) session_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ApprovalRequest {
    pub(super) id: Option<String>,
    pub(super) approval_id: String,
    pub(super) approved: bool,
    pub(super) note: Option<String>,
    #[serde(default)]
    pub(super) cache: ApprovalCacheScope,
}

#[derive(Debug, Deserialize)]
pub(super) struct UserInputRequest {
    pub(super) id: Option<String>,
    pub(super) request_id: String,
    pub(super) response: UserInputResponse,
}

#[derive(Debug, Deserialize)]
pub(super) struct CancelRequest {
    pub(super) id: Option<String>,
    pub(super) target_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetPermissionModeRequest {
    pub(super) id: Option<String>,
    pub(super) mode: PermissionMode,
    #[serde(default)]
    pub(super) session_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetModelRequest {
    pub(super) id: Option<String>,
    pub(super) model: String,
    #[serde(default)]
    pub(super) session_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetReasoningEffortRequest {
    pub(super) id: Option<String>,
    pub(super) effort: Option<String>,
    #[serde(default)]
    pub(super) session_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetReasoningEnabledRequest {
    pub(super) id: Option<String>,
    pub(super) enabled: bool,
    #[serde(default)]
    pub(super) session_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetConfigBuilderRequest {
    #[serde(default)]
    pub(super) modules: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) module_config: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    /// `None` — не трогать `tools.enabled`; `Some` — заменить список целиком.
    #[serde(default)]
    pub(super) tools_enabled: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetWebConfigRequest {
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) tool_cards_collapsed: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResumeSessionRequest {
    pub(super) id: Option<String>,
    pub(super) session_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
pub(super) struct NewSessionRequest {
    pub(super) id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DeleteSessionRequest {
    pub(super) id: Option<String>,
    pub(super) session_dir: PathBuf,
}
