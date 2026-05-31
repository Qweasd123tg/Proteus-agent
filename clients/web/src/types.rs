use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PermissionMode {
    Plan,
    Normal,
    Auto,
}

impl PermissionMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Normal => "normal",
            Self::Auto => "auto",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Plan => "только чтение",
            Self::Normal => "спрашивать перед записью",
            Self::Auto => "писать без запросов",
        }
    }

    pub(crate) fn from_value(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "plan" => Self::Plan,
            "auto" => Self::Auto,
            _ => Self::Normal,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum ReasoningEffort {
    #[default]
    Config,
    Custom(String),
}

impl ReasoningEffort {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Config => "auto".to_owned(),
            Self::Custom(value) => value.clone(),
        }
    }

    pub(crate) fn value(&self) -> String {
        match self {
            Self::Config => "auto".to_owned(),
            Self::Custom(value) => value.clone(),
        }
    }

    pub(crate) fn effort(&self) -> Option<String> {
        match self {
            Self::Config => None,
            Self::Custom(value) => Some(value.clone()),
        }
    }

    pub(crate) fn from_value(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty()
            || value.eq_ignore_ascii_case("auto")
            || value.eq_ignore_ascii_case("config")
        {
            Self::Config
        } else {
            Self::Custom(value.to_owned())
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApprovalCacheScope {
    #[default]
    None,
    ExactCall,
    ToolInCwd,
}

impl ApprovalCacheScope {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "Один раз",
            Self::ExactCall => "Точно",
            Self::ToolInCwd => "Tool/CWD",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MessageRole {
    User,
    Assistant,
    System,
}

impl MessageRole {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::User => "Вы",
            Self::Assistant => "Proteus",
            Self::System => "Система",
        }
    }

    pub(crate) fn card_class(&self) -> &'static str {
        match self {
            Self::User => "task-card",
            Self::Assistant => "task-card success",
            Self::System => "task-card running",
        }
    }

    pub(crate) fn message_class(&self) -> &'static str {
        match self {
            Self::User => "message user-message",
            Self::Assistant => "message assistant-message",
            Self::System => "message system-message",
        }
    }

    pub(crate) fn badge_class(&self) -> &'static str {
        match self {
            Self::User => "status-badge idle",
            Self::Assistant => "status-badge completed",
            Self::System => "status-badge running",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Message {
    pub(crate) id: u64,
    pub(crate) role: MessageRole,
    pub(crate) text: String,
    pub(crate) tool: Option<ToolActivity>,
    pub(crate) streaming: bool,
}

impl Message {
    pub(crate) fn render_key(&self) -> (u64, usize, bool) {
        (self.id, self.text.len(), self.streaming)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToastMessage {
    pub(crate) id: u64,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActivityItem {
    pub(crate) label: &'static str,
    pub(crate) value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolActivity {
    pub(crate) call_id: String,
    pub(crate) name: String,
    pub(crate) args_preview: String,
    pub(crate) status: ToolActivityStatus,
    pub(crate) result_preview: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolActivityStatus {
    Running,
    WaitingApproval,
    Approved,
    Denied,
    Done,
    Failed,
}

impl ToolActivityStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Running => "выполняется",
            Self::WaitingApproval => "ждёт доступ",
            Self::Approved => "разрешено",
            Self::Denied => "отклонено",
            Self::Done => "готово",
            Self::Failed => "ошибка",
        }
    }

    pub(crate) fn badge_class(self) -> &'static str {
        match self {
            Self::Running | Self::WaitingApproval | Self::Approved => "status-badge running",
            Self::Done => "status-badge completed",
            Self::Denied | Self::Failed => "status-badge failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ToolCallInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) args: Value,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ApprovalRequestInfo {
    pub(crate) approval_id: String,
    pub(crate) call: ToolCallInfo,
    pub(crate) cwd: String,
    pub(crate) reason: String,
    pub(crate) tool_spec: Option<Value>,
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

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct SessionSummary {
    pub(crate) session_dir: String,
    pub(crate) session_id: Option<String>,
    pub(crate) workspace_path: Option<String>,
    pub(crate) message_count: usize,
    pub(crate) updated_at_ms: Option<u64>,
    pub(crate) preview: Option<String>,
    pub(crate) resumable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct TranscriptMessage {
    pub(crate) role: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TransportStatus {
    Connecting,
    Connected,
    Error(String),
    Shutdown,
}

impl TransportStatus {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Connecting => "подключение".to_owned(),
            Self::Connected => "подключено".to_owned(),
            Self::Error(message) => format!("ошибка: {message}"),
            Self::Shutdown => "остановлено".to_owned(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StdioOutput {
    Event {
        event: AppServerEvent,
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
    Runtime {
        envelope: Value,
    },
    UserMessageSubmitted {
        text: String,
    },
    TurnOutput {
        output: Value,
    },
    ApprovalRequested {
        request: ApprovalRequestInfo,
    },
    ApprovalResolved {
        approval_id: String,
        approved: bool,
    },
    UserInputRequested {
        request: UserInputRequestInfo,
    },
    UserInputResolved {
        request_id: String,
    },
    Error {
        message: String,
    },
    Shutdown,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize)]
pub(crate) struct SendRequest {
    pub(crate) id: Option<String>,
    pub(crate) text: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SetPermissionModeRequest {
    pub(crate) id: Option<String>,
    pub(crate) mode: PermissionMode,
}

#[derive(Debug, Serialize)]
pub(crate) struct SetModelRequest {
    pub(crate) id: Option<String>,
    pub(crate) model: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SetReasoningEffortRequest {
    pub(crate) id: Option<String>,
    pub(crate) effort: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SetReasoningEnabledRequest {
    pub(crate) id: Option<String>,
    pub(crate) enabled: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResolveApprovalRequest {
    pub(crate) id: Option<String>,
    pub(crate) approval_id: String,
    pub(crate) approved: bool,
    pub(crate) note: Option<String>,
    #[serde(default)]
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
