use super::AppApprovalRequest;
use crate::{
    contracts::UserInputRequest,
    domain::{MessageId, SessionId},
};
use serde::{Deserialize, Serialize};

/// Snapshot текущих интерактивных запросов app-server'а. UI использует его
/// после reconnect/initial load, чтобы восстановить approval и typed input
/// карточки, если live SSE event был пропущен.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AppPendingRequests {
    pub session_id: SessionId,
    /// Новая идентичность при каждом запуске live app session. Seq сравнивается
    /// только внутри этого stream, включая cold resume того же session_id.
    pub stream_id: String,
    pub seq: u64,
    pub approvals: Vec<AppApprovalRequest>,
    pub user_inputs: Vec<UserInputRequest>,
    pub queued_user_messages: Vec<AppQueuedUserMessage>,
}

impl AppPendingRequests {
    pub fn new(session_id: SessionId, stream_id: String) -> Self {
        Self {
            session_id,
            stream_id,
            seq: 0,
            approvals: Vec::new(),
            user_inputs: Vec::new(),
            queued_user_messages: Vec::new(),
        }
    }

    pub fn with_queued_user_messages(mut self, messages: Vec<AppQueuedUserMessage>) -> Self {
        self.queued_user_messages = messages;
        self
    }
}

/// User message, принятый сервером во время активного root turn-а, но ещё не
/// доставленный модели. Снимок используется `/pending` после reconnect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AppQueuedUserMessage {
    pub message_id: MessageId,
    pub text: String,
}

impl AppQueuedUserMessage {
    pub fn new(message_id: MessageId, text: impl Into<String>) -> Self {
        Self {
            message_id,
            text: text.into(),
        }
    }
}
