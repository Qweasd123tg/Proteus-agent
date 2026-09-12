//! Wire protocol для AppServer: события и команды, которыми общаются
//! ядро (`proteus server stdio` / `proteus server http`) и внешние
//! web/desktop-клиенты.
//!
//! Клиенты depend на этот модуль (через `proteus-contracts`), **не** на
//! само ядро (`proteus-core`). Это сохраняет архитектурную границу без
//! обещания совместимости между черновыми версиями wire-контракта.
//!
//! ## Формат transport
//!
//! `proteus server stdio` читает по одной JSONL-строке `StdioRequest` из
//! stdin и пишет по одной JSONL-строке `StdioOutput` в stdout.
//! `proteus server http` принимает тот же `StdioRequest` через `POST /request`
//! и публикует `StdioOutput::Event` через `GET /events` как SSE. Сессионные
//! HTTP-команды `/request` и подписка требуют явный `?session_dir=...`; stdio
//! привязан к сессии запуском процесса. Оба формата
//! используют tagged enum с полем `"type"`.
//!
//! ## Жизненный цикл
//!
//! До стабилизации server и клиенты обновляются вместе. Удалённые поля,
//! enum-values и старые payload shapes не распознаются и не понижаются до
//! defaults: несовпадение контракта должно завершаться явной decode-ошибкой.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    contracts::UserInputRequest,
    domain::{AgentOutput, EventEnvelope},
};

pub mod config;
pub mod config_builder;
pub mod http;
mod live;
pub mod topology;
mod transcript;
pub use live::{AppExecutionState, AppRun, AppRunStatus, AppSessionSnapshot};
pub use transcript::{AppTranscriptMessage, AppTranscriptSubagent, AppTranscriptTool};

mod pending;
pub use pending::{AppPendingRequests, AppQueuedUserMessage};
mod requests;
mod session;
pub use requests::StdioRequest;

pub use session::{AppBootstrap, AppSessionActivity, AppSessionActivityStatus, AppSessionSummary};

/// ID approval'а — произвольная строка, уникальная для session агента.
pub type AppApprovalId = String;
pub type AppUserInputRequestId = String;

/// События, которые ядро публикует внешним клиентам.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppServerEvent {
    /// Initial or replacement state, followed only by events after its seq.
    SessionSnapshot { snapshot: Box<AppSessionSnapshot> },
    /// Authoritative execution lifecycle, including cancellation requested.
    ExecutionUpdated { execution: AppExecutionState },

    /// Runtime-событие с полным envelope. UI использует его для
    /// прогресс-индикации, timeline/replay и correlation по event/turn ids.
    Runtime { envelope: Box<EventEnvelope> },

    /// Пользователь отправил текстовое сообщение (echo обратно клиенту).
    UserMessageSubmitted { text: String },

    /// Финальный AgentOutput после завершения turn'а.
    TurnOutput { output: Box<AgentOutput> },

    /// Полное состояние очереди и интерактивных запросов на одной версии.
    /// Snapshot и HTTP `/pending` имеют одинаковую форму и порядок `seq`.
    PendingRequestsUpdated { snapshot: Box<AppPendingRequests> },

    /// Запрос на approval от модели. Клиент должен показать пользователю
    /// и ответить через `StdioRequest::Approval`.
    ApprovalRequested { request: Box<AppApprovalRequest> },

    /// Approval разрешён (через любой источник: клиент, timeout, shutdown).
    ApprovalResolved {
        approval_id: AppApprovalId,
        approved: bool,
    },

    /// Запрос typed user input от tool `request_user_input`.
    UserInputRequested { request: Box<UserInputRequest> },

    /// User-input request разрешён клиентом, timeout'ом или shutdown'ом.
    UserInputResolved { request_id: AppUserInputRequestId },

    /// Runtime опубликовал новый snapshot модулей/tools. Уже активные turns
    /// продолжают работать на старом epoch, новые turns берут новый.
    ModulesReloaded {
        old_epoch: u64,
        new_epoch: u64,
        tool_names: Vec<String>,
    },

    /// App-server обновил control-plane состояние session. Это событие не
    /// несёт transcript/runtime deltas и может приходить для фоновой session,
    /// чтобы клиенты могли подсветить running/pending чат в sidebar.
    SessionActivityUpdated {
        session_dir: PathBuf,
        activity: AppSessionActivity,
    },

    /// Ошибка в turn или ядре.
    Error { message: String },

    /// Поток событий отстал и часть событий потеряна (переполнение broadcast
    /// ring на стороне сервера, например при заторможенном клиенте). Клиент
    /// отбрасывает buffered deltas и ждёт следующего SessionSnapshot из той
    /// же подписки. Pending имеет независимую полную watch projection.
    EventStreamLagged { count: u64 },

    /// Ядро завершило работу. Клиент должен выйти.
    Shutdown,
}

mod approval;
pub use approval::{AppApprovalPreview, AppApprovalRequest};

mod context;
pub use context::*;

/// Короткий ответ для line-oriented клиентов, которым не нужна полная
/// transcript projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppHistorySummary {
    pub messages: usize,
}

impl AppHistorySummary {
    pub fn new(messages: usize) -> Self {
        Self { messages }
    }
}

/// Результат явной записи пользователя в выбранный `MemoryStore`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppRememberResult {
    pub kind: String,
    pub content: String,
}

impl AppRememberResult {
    pub fn new(kind: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            content: content.into(),
        }
    }
}

/// Выход ядра — события и ответы на команды.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StdioOutput {
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

#[cfg(test)]
mod tests;
