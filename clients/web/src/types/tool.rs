use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolActivity {
    pub(crate) call_id: String,
    pub(crate) name: String,
    pub(crate) args: Value,
    pub(crate) args_preview: String,
    pub(crate) started_at_ms: u64,
    /// Момент терминального статуса (done/failed/denied) — для duration в
    /// карточке. None у бегущих и у восстановленных из истории (там момента
    /// старта нет, duration не считается).
    pub(crate) finished_at_ms: Option<u64>,
    pub(crate) status: ToolActivityStatus,
    pub(crate) result_preview: Option<String>,
}

impl ToolActivity {
    /// Длительность выполнения в миллисекундах, если известны обе границы.
    pub(crate) fn duration_ms(&self) -> Option<u64> {
        if self.started_at_ms == 0 {
            return None;
        }
        self.finished_at_ms
            .map(|finished| finished.saturating_sub(self.started_at_ms))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolActivityStatus {
    Running,
    WaitingApproval,
    Approved,
    Denied,
    Done,
    Failed,
    /// Ход закончился (или история восстановлена без результата), а
    /// терминального события у вызова нет — спиннер не должен жить вечно.
    Interrupted,
}

impl ToolActivityStatus {
    /// Терминальный статус: карточка больше не изменит состояние сама.
    pub(crate) fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed | Self::Denied | Self::Interrupted
        )
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
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
            Self::Interrupted => "прервано",
        }
    }

    pub(crate) fn badge_class(self) -> &'static str {
        match self {
            Self::Running | Self::WaitingApproval | Self::Approved => "status-badge running",
            Self::Done => "status-badge completed",
            Self::Denied | Self::Failed => "status-badge failed",
            Self::Interrupted => "status-badge idle",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ToolCallInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) args: Value,
}
