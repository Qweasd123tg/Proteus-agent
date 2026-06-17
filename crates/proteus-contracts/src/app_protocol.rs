//! Wire protocol для AppServer: события и команды, которыми общаются
//! ядро (`proteus server stdio` / `proteus server http`) и внешние
//! web/desktop-клиенты.
//!
//! Клиенты depend на этот модуль (через `proteus-contracts`), **не** на
//! само ядро (`proteus-core`). Это даёт архитектурную границу: любой
//! клиент можно собирать независимо и обновлять отдельно от ядра,
//! совместимость определяется версией `proteus-contracts`.
//!
//! ## Формат transport
//!
//! `proteus server stdio` читает по одной JSONL-строке `StdioRequest` из
//! stdin и пишет по одной JSONL-строке `StdioOutput` в stdout.
//! `proteus server http` принимает тот же `StdioRequest` через `POST /request`
//! и публикует `StdioOutput::Event` через `GET /events` как SSE. Оба формата
//! используют tagged enum с полем `"type"`.
//!
//! ## Стабильность
//!
//! Все публичные структуры помечены `#[non_exhaustive]` — добавление
//! полей не ломает существующих клиентов, они игнорируют незнакомые поля.
//! Enum-поля должны отдельно поддерживать tolerant parsing, если их значения
//! могут расширяться без синхронного обновления всех клиентов. Например,
//! неизвестный `ApprovalCacheScope` намеренно понижается до `none`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    contracts::{ApprovalCacheScope, UserInputRequest, UserInputResponse},
    domain::{AgentOutput, EventEnvelope, PermissionMode, ToolCall, ToolSpec},
};

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
    Runtime { envelope: EventEnvelope },

    /// Пользователь отправил текстовое сообщение (echo обратно клиенту).
    UserMessageSubmitted { text: String },

    /// Финальный AgentOutput после завершения turn'а.
    TurnOutput { output: AgentOutput },

    /// Запрос на approval от модели. Клиент должен показать пользователю
    /// и ответить через `StdioRequest::Approval`.
    ApprovalRequested { request: AppApprovalRequest },

    /// Approval разрешён (через любой источник: клиент, timeout, shutdown).
    ApprovalResolved {
        approval_id: AppApprovalId,
        approved: bool,
    },

    /// Запрос typed user input от tool `request_user_input`.
    UserInputRequested { request: UserInputRequest },

    /// User-input request разрешён клиентом, timeout'ом или shutdown'ом.
    UserInputResolved { request_id: AppUserInputRequestId },

    /// Runtime опубликовал новый snapshot модулей/tools. Уже активные turns
    /// продолжают работать на старом epoch, новые turns берут новый.
    ModulesReloaded {
        old_epoch: u64,
        new_epoch: u64,
        tool_names: Vec<String>,
    },

    /// Ошибка в turn или ядре.
    Error { message: String },

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
    #[serde(default)]
    pub preview: Option<AppApprovalPreview>,
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
        }
    }

    pub fn with_preview(mut self, preview: Option<AppApprovalPreview>) -> Self {
        self.preview = preview;
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
    #[serde(default)]
    pub affected_files: Vec<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
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
        #[serde(default)]
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
    use crate::domain::new_call_id;

    #[test]
    fn approval_request_defaults_missing_preview_to_none() {
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

        let request: AppApprovalRequest =
            serde_json::from_value(payload).expect("approval request");

        assert_eq!(request.approval_id, "approval-1");
        assert!(request.preview.is_none());
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
}
