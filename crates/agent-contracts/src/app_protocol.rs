//! Wire protocol для AppServer: события и команды, которыми общаются
//! ядро (`agent server stdio`) и внешние клиенты (TUI, GUI, web).
//!
//! Клиенты depend на этот модуль (через `agent-contracts`), **не** на
//! само ядро (`modular-agent`). Это даёт архитектурную границу: любой
//! клиент можно собирать независимо и обновлять отдельно от ядра,
//! совместимость определяется версией `agent-contracts`.
//!
//! ## Формат transport
//!
//! `agent server stdio` читает по одной JSONL-строке `StdioRequest` из
//! stdin и пишет по одной JSONL-строке `StdioOutput` в stdout. Оба —
//! tagged enum с полем `"type"`.
//!
//! ## Стабильность
//!
//! Все публичные структуры помечены `#[non_exhaustive]` — добавление
//! полей и вариантов не ломает существующих клиентов, они игнорируют
//! незнакомые поля.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    contracts::ApprovalCacheScope,
    domain::{AgentOutput, Event, ToolCall, ToolSpec},
};

/// ID approval'а — произвольная строка, уникальная для session агента.
pub type AppApprovalId = String;

/// События, которые ядро публикует внешним клиентам.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AppServerEvent {
    /// Сырое runtime-событие из доменного слоя (TaskReceived, ToolFinished
    /// и т.п.). UI использует для прогресс-индикации.
    Runtime { event: Event },

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
        }
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
    Cancel {
        id: Option<String>,
        target_id: String,
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
            | Self::Cancel { id, .. }
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
