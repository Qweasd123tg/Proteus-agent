use std::collections::HashMap;

use serde::Serialize;

use super::{ApprovalCacheScope, PermissionMode};

#[derive(Debug, Serialize)]
pub(crate) struct SendRequest {
    pub(crate) id: Option<String>,
    pub(crate) text: String,
    pub(crate) session_dir: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct EditQueuedMessageRequest {
    pub(crate) id: Option<String>,
    pub(crate) session_dir: String,
    pub(crate) message_id: String,
    pub(crate) text: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct DeleteQueuedMessageRequest {
    pub(crate) id: Option<String>,
    pub(crate) session_dir: String,
    pub(crate) message_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SetPermissionModeRequest {
    pub(crate) id: Option<String>,
    pub(crate) mode: PermissionMode,
    pub(crate) session_dir: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SetModelRequest {
    pub(crate) id: Option<String>,
    pub(crate) model: String,
    pub(crate) session_dir: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SetReasoningEffortRequest {
    pub(crate) id: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) session_dir: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResolveApprovalRequest {
    pub(crate) id: Option<String>,
    pub(crate) approval_id: String,
    pub(crate) approved: bool,
    pub(crate) note: Option<String>,
    pub(crate) cache: ApprovalCacheScope,
}

#[derive(Debug, Serialize)]
pub(crate) struct UserInputSubmitRequest {
    pub(crate) id: Option<String>,
    pub(crate) request_id: String,
    pub(crate) response: UserInputResponseBody,
}

#[derive(Debug, Serialize)]
pub(crate) struct UserInputResponseBody {
    pub(crate) answers: HashMap<String, UserInputAnswerBody>,
}

#[derive(Debug, Serialize)]
pub(crate) struct UserInputAnswerBody {
    pub(crate) answers: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CancelRequest {
    pub(crate) id: Option<String>,
    pub(crate) target_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResumeSessionRequest {
    pub(crate) id: Option<String>,
    pub(crate) session_dir: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct DeleteSessionRequest {
    pub(crate) id: Option<String>,
    pub(crate) session_dir: String,
}
