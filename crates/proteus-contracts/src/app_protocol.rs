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
//! и публикует `StdioOutput::Event` через `GET /events` как SSE. Оба формата
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
    contracts::{ApprovalCacheScope, RequestOrigin, UserInputRequest, UserInputResponse},
    domain::{
        AgentOutput, EventEnvelope, HistoryCompactionReport, MessageId, PermissionMode, SessionId,
        ToolCall, ToolSpec, TurnId,
    },
    model_standard::TokenUsage,
};

mod session;

pub use session::{AppSessionActivity, AppSessionActivityStatus, AppSessionSummary};

/// ID approval'а — произвольная строка, уникальная для session агента.
pub type AppApprovalId = String;
pub type AppUserInputRequestId = String;

/// События, которые ядро публикует внешним клиентам.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AppServerEvent {
    /// Runtime-событие с полным envelope. UI использует его для
    /// прогресс-индикации, timeline/replay и correlation по event/turn ids.
    Runtime { envelope: Box<EventEnvelope> },

    /// Пользователь отправил текстовое сообщение (echo обратно клиенту).
    UserMessageSubmitted { text: String },

    /// Финальный AgentOutput после завершения turn'а.
    TurnOutput { output: Box<AgentOutput> },

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
    /// обязан считать стрим-состояние невалидным и пересинхронизировать
    /// transcript/pending с сервера (`/history`, `/pending`) — среди
    /// потерянных событий могли быть `ToolFinished`/`TurnOutput`.
    EventStreamLagged { count: u64 },

    /// Ядро завершило работу. Клиент должен выйти.
    Shutdown,
}

/// Approval request, адресованный клиенту.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AppApprovalRequest {
    pub approval_id: AppApprovalId,
    pub call: ToolCall,
    pub cwd: PathBuf,
    pub reason: String,
    pub tool_spec: Option<ToolSpec>,
    pub preview: Option<AppApprovalPreview>,
    /// Кто запросил approval (thread/turn + метка источника, например роль
    /// субагента). `None` — запрос от транспорта без runtime-атрибуции.
    pub origin: Option<RequestOrigin>,
    /// Монотонный порядковый номер в очереди pending approvals текущего
    /// app-server. Клиенты сортируют по нему.
    pub seq: u64,
}

impl AppApprovalRequest {
    pub fn new(
        approval_id: AppApprovalId,
        call: ToolCall,
        cwd: PathBuf,
        reason: String,
        tool_spec: Option<ToolSpec>,
    ) -> Self {
        Self {
            approval_id,
            call,
            cwd,
            reason,
            tool_spec,
            preview: None,
            origin: None,
            seq: 0,
        }
    }

    pub fn with_preview(mut self, preview: Option<AppApprovalPreview>) -> Self {
        self.preview = preview;
        self
    }

    pub fn with_origin(mut self, origin: Option<RequestOrigin>) -> Self {
        self.origin = origin;
        self
    }

    pub fn with_seq(mut self, seq: u64) -> Self {
        self.seq = seq;
        self
    }
}

/// UI-oriented approval preview. It is advisory only: actual execution must
/// still go through the tool's own validation and policy checks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct AppApprovalPreview {
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub affected_files: Vec<String>,
    pub body: Option<String>,
    pub language: Option<String>,
    pub metadata: Value,
}

impl AppApprovalPreview {
    pub fn new(
        kind: impl Into<String>,
        title: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            title: title.into(),
            summary: summary.into(),
            affected_files: Vec::new(),
            body: None,
            language: None,
            metadata: Value::Null,
        }
    }

    pub fn with_affected_files(mut self, affected_files: Vec<String>) -> Self {
        self.affected_files = affected_files;
        self
    }

    pub fn with_body(mut self, body: impl Into<String>, language: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self.language = Some(language.into());
        self
    }

    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Snapshot текущих интерактивных запросов app-server'а. UI использует его
/// после reconnect/initial load, чтобы восстановить approval и typed input
/// карточки, если live SSE event был пропущен.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct AppPendingRequests {
    pub approvals: Vec<AppApprovalRequest>,
    pub user_inputs: Vec<UserInputRequest>,
    pub queued_user_messages: Vec<AppQueuedUserMessage>,
}

impl AppPendingRequests {
    pub fn new(approvals: Vec<AppApprovalRequest>, user_inputs: Vec<UserInputRequest>) -> Self {
        Self {
            approvals,
            user_inputs,
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

/// Диагностический snapshot того, что известно app-server'у о контексте
/// выбранной session. Это UI/debug surface: provider `TokenUsage` остаётся
/// source of truth для totals, а category breakdown смешивает локальную оценку
/// prompt parts с явно помеченной provider telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AppContextMapSnapshot {
    pub session_dir: Option<PathBuf>,
    pub session_id: Option<SessionId>,
    pub workspace_path: Option<PathBuf>,
    pub activity: Option<AppSessionActivity>,
    pub history: AppContextHistorySummary,
    pub latest_usage: Option<AppContextUsageSnapshot>,
    pub latest_context: Option<AppContextBuildSnapshot>,
    pub latest_compaction: Option<AppContextCompactionSnapshot>,
    pub tools: AppContextToolSummary,
    pub diagnostics: Vec<String>,
}

impl AppContextMapSnapshot {
    pub fn new(
        session_dir: Option<PathBuf>,
        session_id: Option<SessionId>,
        workspace_path: Option<PathBuf>,
        history: AppContextHistorySummary,
        tools: AppContextToolSummary,
    ) -> Self {
        Self {
            session_dir,
            session_id,
            workspace_path,
            activity: None,
            history,
            latest_usage: None,
            latest_context: None,
            latest_compaction: None,
            tools,
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppContextHistorySummary {
    pub messages: usize,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub system_messages: usize,
    pub tool_results: usize,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppContextUsageSnapshot {
    pub model_provider: String,
    pub model_name: String,
    pub phase: Option<String>,
    pub estimated_input_tokens: u32,
    pub max_input_tokens: Option<u32>,
    pub compaction_trigger_tokens: Option<u32>,
    pub categories: Vec<AppContextUsageCategory>,
    pub actual: Option<TokenUsage>,
    pub source: String,
    pub turn_id: Option<TurnId>,
    pub timestamp_ms: Option<i64>,
}

impl AppContextUsageSnapshot {
    pub fn new(
        model_provider: impl Into<String>,
        model_name: impl Into<String>,
        estimated_input_tokens: u32,
        source: impl Into<String>,
    ) -> Self {
        Self {
            model_provider: model_provider.into(),
            model_name: model_name.into(),
            phase: None,
            estimated_input_tokens,
            max_input_tokens: None,
            compaction_trigger_tokens: None,
            categories: Vec::new(),
            actual: None,
            source: source.into(),
            turn_id: None,
            timestamp_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppContextUsageCategory {
    pub name: String,
    pub tokens: u32,
    pub source: Option<String>,
}

impl AppContextUsageCategory {
    pub fn new(name: impl Into<String>, tokens: u32) -> Self {
        Self {
            name: name.into(),
            tokens,
            source: None,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppContextBuildSnapshot {
    pub chunks: usize,
    pub token_estimate: Option<u32>,
    pub turn_id: Option<TurnId>,
    pub timestamp_ms: Option<i64>,
}

impl AppContextBuildSnapshot {
    pub fn new(chunks: usize) -> Self {
        Self {
            chunks,
            token_estimate: None,
            turn_id: None,
            timestamp_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct AppContextCompactionSnapshot {
    pub status: String,
    pub report: Option<HistoryCompactionReport>,
    pub summary_present: bool,
    pub turn_id: Option<TurnId>,
    pub timestamp_ms: Option<i64>,
}

impl AppContextCompactionSnapshot {
    pub fn new(status: impl Into<String>) -> Self {
        Self {
            status: status.into(),
            report: None,
            summary_present: false,
            turn_id: None,
            timestamp_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppContextToolSummary {
    pub requested: usize,
    pub finished: usize,
    pub failed: usize,
    pub names: Vec<String>,
}

/// Команды от клиента к ядру через stdin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum StdioRequest {
    Send {
        id: Option<String>,
        text: String,
    },
    ClearHistory {
        id: Option<String>,
    },
    Approval {
        id: Option<String>,
        approval_id: String,
        approved: bool,
        note: Option<String>,
        cache: ApprovalCacheScope,
    },
    UserInput {
        id: Option<String>,
        request_id: String,
        response: UserInputResponse,
    },
    Cancel {
        id: Option<String>,
        target_id: String,
    },
    SetPermissionMode {
        id: Option<String>,
        mode: PermissionMode,
    },
    SetModel {
        id: Option<String>,
        model: String,
    },
    SetReasoningEffort {
        id: Option<String>,
        effort: Option<String>,
    },
    SetReasoningEnabled {
        id: Option<String>,
        enabled: bool,
    },
    ConfigSummary {
        id: Option<String>,
    },
    ReloadTools {
        id: Option<String>,
    },
    Shutdown {
        id: Option<String>,
    },
}

impl StdioRequest {
    pub fn id(&self) -> Option<String> {
        match self {
            Self::Send { id, .. }
            | Self::ClearHistory { id }
            | Self::Approval { id, .. }
            | Self::UserInput { id, .. }
            | Self::Cancel { id, .. }
            | Self::SetPermissionMode { id, .. }
            | Self::SetModel { id, .. }
            | Self::SetReasoningEffort { id, .. }
            | Self::SetReasoningEnabled { id, .. }
            | Self::ConfigSummary { id }
            | Self::ReloadTools { id }
            | Self::Shutdown { id } => id.clone(),
        }
    }
}

/// Выход ядра — события и ответы на команды.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
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
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::domain::{
        Event, EventContext, EventEnvelope, new_call_id, new_session_id, new_thread_id, new_turn_id,
    };

    #[test]
    fn approval_request_rejects_incomplete_wire_payload() {
        let payload = json!({
            "approval_id": "approval-1",
            "call": {
                "id": "call-1",
                "name": "shell",
                "args": { "command": "cargo test" }
            },
            "cwd": "/workspace",
            "reason": "test approval",
            "tool_spec": null
        });

        serde_json::from_value::<AppApprovalRequest>(payload)
            .expect_err("incomplete approval request must fail");
    }

    /// Attribution и порядок очереди переживают wire-сериализацию.
    #[test]
    fn approval_request_roundtrips_origin_and_seq() {
        let origin = crate::contracts::RequestOrigin::new(new_thread_id(), new_turn_id())
            .with_label("explore");
        let request = AppApprovalRequest::new(
            "approval-1".to_owned(),
            ToolCall::new(new_call_id(), "shell", json!({ "command": "cargo test" })),
            PathBuf::from("/workspace"),
            "test approval".to_owned(),
            None,
        )
        .with_origin(Some(origin.clone()))
        .with_seq(42);

        let payload = serde_json::to_value(&request).expect("serialize approval request");
        let parsed: AppApprovalRequest =
            serde_json::from_value(payload).expect("parse approval request");

        assert_eq!(parsed.origin, Some(origin));
        assert_eq!(parsed.seq, 42);
    }

    /// User-input запросы несут ту же attribution/queue-position схему, что
    /// approvals.
    #[test]
    fn user_input_request_roundtrips_origin_and_seq() {
        let origin = crate::contracts::RequestOrigin::new(new_thread_id(), new_turn_id())
            .with_label("explore");
        let request = UserInputRequest::new("input-1", PathBuf::from("/workspace"), Vec::new())
            .with_origin(origin.clone())
            .with_seq(7);

        let payload = serde_json::to_value(&request).expect("serialize user input request");
        let parsed: UserInputRequest =
            serde_json::from_value(payload).expect("parse user input request");
        assert_eq!(parsed.origin, Some(origin));
        assert_eq!(parsed.seq, 7);
    }

    #[test]
    fn approval_request_roundtrips_preview() {
        let request = AppApprovalRequest::new(
            "approval-1".to_owned(),
            ToolCall::new(
                new_call_id(),
                "write_file",
                json!({ "path": "a.txt", "content": "hello" }),
            ),
            PathBuf::from("/workspace"),
            "test approval".to_owned(),
            None,
        )
        .with_preview(Some(
            AppApprovalPreview::new("write_file", "File write preview", "Create a.txt")
                .with_affected_files(vec!["a.txt".to_owned()])
                .with_body("hello", "text")
                .with_metadata(json!({ "operation": "create" })),
        ));

        let value = serde_json::to_value(&request).expect("serialize request");
        let decoded: AppApprovalRequest = serde_json::from_value(value).expect("decode request");

        let preview = decoded.preview.expect("preview");
        assert_eq!(preview.kind, "write_file");
        assert_eq!(preview.affected_files, vec!["a.txt"]);
        assert_eq!(preview.metadata["operation"], "create");
    }

    #[test]
    fn boxed_app_server_event_keeps_wire_shape() {
        let session_id = new_session_id();
        let thread_id = new_thread_id();
        let event = AppServerEvent::Runtime {
            envelope: Box::new(EventEnvelope::new(
                EventContext::new(session_id, thread_id, None),
                1,
                Event::SessionStarted {
                    session_id,
                    cwd: PathBuf::from("/workspace"),
                    model: None,
                    session_dir: None,
                },
            )),
        };

        let value = serde_json::to_value(event).expect("event JSON");

        assert_eq!(value["type"], "runtime");
        assert_eq!(value["envelope"]["seq"], 1);

        let decoded: AppServerEvent = serde_json::from_value(value).expect("decode event");
        match decoded {
            AppServerEvent::Runtime { envelope } => assert_eq!(envelope.seq, 1),
            other => panic!("expected runtime event, got {other:?}"),
        }
    }

    #[test]
    fn session_activity_uses_stable_status_order() {
        assert_eq!(
            AppSessionActivity::from_counts(0, 0, 0).status,
            AppSessionActivityStatus::Idle
        );
        assert_eq!(
            AppSessionActivity::from_counts(1, 0, 0).status,
            AppSessionActivityStatus::Running
        );
        assert_eq!(
            AppSessionActivity::from_counts(1, 1, 0).status,
            AppSessionActivityStatus::WaitingApproval
        );
        assert_eq!(
            AppSessionActivity::from_counts(1, 1, 1).status,
            AppSessionActivityStatus::WaitingInput
        );
    }

    #[test]
    fn session_activity_status_stays_string_on_wire() {
        let activity = AppSessionActivity::from_counts(1, 0, 0);
        let value = serde_json::to_value(activity).expect("activity JSON");

        assert_eq!(value["status"], "running");
    }

    #[test]
    fn session_activity_can_carry_running_turn_ids() {
        let activity = AppSessionActivity::from_running_turn_ids(
            vec!["turn-2".to_owned(), "turn-1".to_owned()],
            0,
            0,
        );
        let value = serde_json::to_value(&activity).expect("activity JSON");

        assert_eq!(activity.running_turns, 2);
        assert_eq!(
            value["running_turn_ids"],
            serde_json::json!(["turn-2", "turn-1"])
        );

        let decoded: AppSessionActivity = serde_json::from_value(value).expect("activity decode");
        assert_eq!(
            decoded.running_turn_ids,
            vec!["turn-2".to_owned(), "turn-1".to_owned()]
        );
    }

    #[test]
    fn session_activity_status_rejects_unknown_wire_value() {
        serde_json::from_value::<AppSessionActivity>(serde_json::json!({
            "status": "paused",
            "running_turns": 0,
            "running_turn_ids": [],
            "pending_approvals": 0,
            "pending_user_inputs": 0,
        }))
        .expect_err("unknown activity status must fail");
    }
}
