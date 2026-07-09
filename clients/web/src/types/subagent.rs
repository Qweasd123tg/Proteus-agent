use super::ToolActivity;

/// Карточка работы субагента в ленте: создаётся по `SubagentStarted`,
/// закрывается по `SubagentFinished`. Tool-вызовы дочернего цикла приходят
/// под `child_thread_id` (см. contracts/subagent.rs) и складываются в `tools`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SubagentActivity {
    pub(crate) child_thread_id: String,
    pub(crate) role: String,
    pub(crate) description: Option<String>,
    pub(crate) status: SubagentActivityStatus,
    /// Число итераций дочернего цикла из `SubagentFinished`.
    pub(crate) iterations: Option<u32>,
    pub(crate) started_at_ms: u64,
    /// Момент `SubagentFinished` — для итоговой длительности. None у бегущих
    /// и восстановленных из истории карточек (там старт неизвестен).
    pub(crate) finished_at_ms: Option<u64>,
    pub(crate) tools: Vec<ToolActivity>,
}

impl SubagentActivity {
    pub(crate) fn is_running(&self) -> bool {
        matches!(self.status, SubagentActivityStatus::Running)
    }

    /// Полная длительность работы, если известны обе границы.
    pub(crate) fn duration_ms(&self) -> Option<u64> {
        if self.started_at_ms == 0 {
            return None;
        }
        self.finished_at_ms
            .map(|finished| finished.saturating_sub(self.started_at_ms))
    }
}

/// Статус карточки: пока идёт дочерний цикл — Running, после
/// `SubagentFinished` — snake_case статус из события ("completed",
/// "cancelled", "timed_out", "max_iterations_reached", "errored", ...).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SubagentActivityStatus {
    Running,
    Finished(String),
}

impl SubagentActivityStatus {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Running => "выполняется".to_owned(),
            Self::Finished(status) => match status.as_str() {
                "completed" => "готово".to_owned(),
                "cancelled" => "отменён".to_owned(),
                "timed_out" => "таймаут".to_owned(),
                "max_iterations_reached" => "лимит итераций".to_owned(),
                "token_budget_exceeded" => "бюджет токенов".to_owned(),
                "errored" => "ошибка".to_owned(),
                "interrupted" => "прервано".to_owned(),
                other => other.replace('_', " "),
            },
        }
    }

    pub(crate) fn badge_class(&self) -> &'static str {
        match self {
            Self::Running => "status-badge running",
            Self::Finished(status) => match status.as_str() {
                "completed" => "status-badge completed",
                // Незавершённый результат (лимиты/отмена) — предупреждение,
                // жёсткая ошибка — failed.
                "errored" | "timed_out" => "status-badge failed",
                _ => "status-badge idle",
            },
        }
    }

    /// Класс внешней карточки хода (рейка running/success/error/idle).
    pub(crate) fn turn_state_class(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Finished(status) => match status.as_str() {
                "completed" => "success",
                "errored" | "timed_out" => "error",
                _ => "idle",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_labels_map_known_finished_states() {
        assert_eq!(SubagentActivityStatus::Running.label(), "выполняется");
        assert_eq!(
            SubagentActivityStatus::Finished("completed".to_owned()).label(),
            "готово"
        );
        assert_eq!(
            SubagentActivityStatus::Finished("max_iterations_reached".to_owned()).label(),
            "лимит итераций"
        );
        // Неизвестный статус не теряется, а показывается как есть.
        assert_eq!(
            SubagentActivityStatus::Finished("paused_for_tea".to_owned()).label(),
            "paused for tea"
        );
    }

    #[test]
    fn turn_state_class_reflects_outcome() {
        assert_eq!(
            SubagentActivityStatus::Running.turn_state_class(),
            "running"
        );
        assert_eq!(
            SubagentActivityStatus::Finished("completed".to_owned()).turn_state_class(),
            "success"
        );
        assert_eq!(
            SubagentActivityStatus::Finished("errored".to_owned()).turn_state_class(),
            "error"
        );
        assert_eq!(
            SubagentActivityStatus::Finished("max_iterations_reached".to_owned())
                .turn_state_class(),
            "idle"
        );
    }
}
