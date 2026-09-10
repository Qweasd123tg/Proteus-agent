use serde::Deserialize;
use serde_json::Value;

use super::{ApprovalRequestInfo, SessionActivityInfo, UserInputRequestInfo};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TransportStatus {
    Connecting,
    Connected,
    /// EventSource оборвался и сам ретраит: ошибку не показываем,
    /// пока не истечёт грейс-период (см. events.rs).
    Reconnecting,
    Error(String),
    Shutdown,
}

impl TransportStatus {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Connecting => "подключение".to_owned(),
            Self::Connected => "подключено".to_owned(),
            Self::Reconnecting => "переподключение".to_owned(),
            Self::Error(message) => format!("ошибка: {message}"),
            Self::Shutdown => "остановлено".to_owned(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StdioOutput {
    Event {
        event: Box<AppServerEvent>,
    },
    Response {
        id: Option<String>,
        ok: bool,
        output: Option<Value>,
        error: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AppServerEvent {
    SessionSnapshot {
        snapshot: Box<super::SessionSnapshot>,
    },
    ExecutionUpdated {
        execution: proteus_client_common::execution::ExecutionState,
    },
    Runtime {
        envelope: Value,
    },
    UserMessageSubmitted {
        text: String,
    },
    TurnOutput {
        output: Value,
    },
    PendingRequestsUpdated {
        snapshot: Box<super::PendingControlPlaneInfo>,
    },
    ApprovalRequested {
        request: Box<ApprovalRequestInfo>,
    },
    ApprovalResolved {
        approval_id: String,
        approved: bool,
    },
    UserInputRequested {
        request: Box<UserInputRequestInfo>,
    },
    UserInputResolved {
        request_id: String,
    },
    ModulesReloaded {
        old_epoch: u64,
        new_epoch: u64,
        tool_names: Vec<String>,
    },
    SessionActivityUpdated {
        session_dir: String,
        activity: SessionActivityInfo,
    },
    Error {
        message: String,
    },
    /// Сервер потерял часть событий (переполнение broadcast ring): стрим-
    /// клиент отбрасывает buffered deltas и ждёт SessionSnapshot из подписки.
    EventStreamLagged {
        count: u64,
    },
    Shutdown,
}
