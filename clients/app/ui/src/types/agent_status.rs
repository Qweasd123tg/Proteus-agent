/// What the agent is doing right now, shown in the working row of the chat.
/// Logic compares variants; only `label` produces text.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum AgentStatus {
    #[default]
    Idle,
    Starting,
    PreparingTask,
    CompactingHistory,
    HistoryCompacted,
    CompactionFailed,
    BuildingContext,
    /// The latest reasoning summary heading names the topic.
    Thinking(Option<String>),
    Writing,
    RunningTool {
        subagent: bool,
    },
    Continuing {
        subagent: bool,
    },
    WaitingApproval {
        subagent: bool,
    },
    WaitingAnswer,
    ApprovalResolved(bool),
    SubagentStarted(String),
    SubagentFinished {
        role: String,
        status: String,
    },
    FollowUp(bool),
    ModulesReloaded {
        old_epoch: u64,
        new_epoch: u64,
        tools: usize,
    },
    Running,
    CancelRequested,
    Canceled,
    Timeout,
    Error,
    Stopped,
}

impl AgentStatus {
    /// The run waits for the user; tool progress must not hide that.
    pub(crate) fn is_waiting(&self) -> bool {
        matches!(self, Self::WaitingApproval { .. } | Self::WaitingAnswer)
    }

    pub(crate) fn label(&self) -> String {
        let subagent = |subagent: bool, text: &str| {
            if subagent {
                format!("субагент {text}")
            } else {
                text.to_owned()
            }
        };
        match self {
            Self::Idle => "ожидает".to_owned(),
            Self::Starting => "начинает".to_owned(),
            Self::PreparingTask => "готовит задачу".to_owned(),
            Self::CompactingHistory => "сжимает историю".to_owned(),
            Self::HistoryCompacted => "история сжата".to_owned(),
            Self::CompactionFailed => "сжатие не удалось".to_owned(),
            Self::BuildingContext => "собирает контекст".to_owned(),
            Self::Thinking(None) => "думает".to_owned(),
            Self::Thinking(Some(topic)) => format!("думает · {topic}"),
            Self::Writing => "пишет".to_owned(),
            Self::RunningTool { subagent: nested } => subagent(*nested, "выполняет действие"),
            Self::Continuing { subagent: true } => "субагент работает".to_owned(),
            Self::Continuing { subagent: false } => "продолжает".to_owned(),
            Self::WaitingApproval { subagent: nested } => subagent(*nested, "ждёт разрешения"),
            Self::WaitingAnswer => "ждёт ответа".to_owned(),
            Self::ApprovalResolved(true) => "разрешено".to_owned(),
            Self::ApprovalResolved(false) => "отклонено".to_owned(),
            Self::SubagentStarted(role) => format!("субагент {role} работает"),
            Self::SubagentFinished { role, status } => format!("субагент {role}: {status}"),
            Self::FollowUp(true) => "начинает следующий ход".to_owned(),
            Self::FollowUp(false) => "учитывает уточнение".to_owned(),
            Self::ModulesReloaded {
                old_epoch,
                new_epoch,
                tools,
            } => format!("модули обновлены: epoch {old_epoch} → {new_epoch}, инструментов {tools}"),
            Self::Running => "работает".to_owned(),
            Self::CancelRequested => "отменяется".to_owned(),
            Self::Canceled => "отменено".to_owned(),
            Self::Timeout => "таймаут".to_owned(),
            Self::Error => "ошибка".to_owned(),
            Self::Stopped => "остановлено".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_name_the_subagent_and_the_reasoning_topic() {
        assert_eq!(AgentStatus::Thinking(Some("Изучаю проект".into())).label(), "думает · Изучаю проект");
        assert_eq!(AgentStatus::WaitingApproval { subagent: true }.label(), "субагент ждёт разрешения");
        assert!(AgentStatus::WaitingAnswer.is_waiting());
        assert!(!AgentStatus::Continuing { subagent: false }.is_waiting());
    }
}
