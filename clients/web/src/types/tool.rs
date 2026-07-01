use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolActivity {
    pub(crate) call_id: String,
    pub(crate) name: String,
    pub(crate) args: Value,
    pub(crate) args_preview: String,
    pub(crate) started_at_ms: u64,
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
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

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
