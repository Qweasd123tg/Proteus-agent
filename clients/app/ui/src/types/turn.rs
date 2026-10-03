use proteus_contracts::model_standard::FinishReason;

/// The app-server records a turn that settled without success as a system
/// message `AppServer <status>: <detail>` (`transcript/journal.rs`).
const TURN_SETTLED_PREFIX: &str = "AppServer ";

/// Why a root turn did not end with a complete answer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TurnIssue {
    Failed(Option<String>),
    Canceled,
    TimedOut,
    Truncated,
    Filtered,
}

impl TurnIssue {
    /// Settled turns come from history, so they survive reloads.
    pub(crate) fn from_transcript(text: &str) -> Option<Self> {
        let (status, detail) = text.strip_prefix(TURN_SETTLED_PREFIX)?.split_once(": ")?;
        let detail = detail.trim();
        Some(match status {
            // A stop is the user's own action; its workflow detail is noise.
            "canceled" => Self::Canceled,
            "timeout" => Self::TimedOut,
            "error" if detail.is_empty() || detail == "turn failed" => Self::Failed(None),
            "error" => Self::Failed(Some(detail.to_owned())),
            _ => return None,
        })
    }

    /// The finish reason exists only live. The latest model response of the
    /// turn decides: a workflow may continue past a truncated step.
    pub(crate) fn from_finish(reason: &FinishReason) -> Option<Option<Self>> {
        match reason {
            FinishReason::Length => Some(Some(Self::Truncated)),
            FinishReason::ContentFilter => Some(Some(Self::Filtered)),
            FinishReason::Stop | FinishReason::ToolCalls => Some(None),
            _ => None,
        }
    }

    pub(crate) fn title(&self) -> &'static str {
        match self {
            Self::Failed(_) => "Ход завершился ошибкой",
            Self::Canceled => "Ход остановлен",
            Self::TimedOut => "Ход остановлен по таймауту",
            Self::Truncated => "Ответ оборван: модель достигла лимита длины ответа",
            Self::Filtered => "Ответ остановлен фильтром содержимого провайдера",
        }
    }

    pub(crate) fn hint(&self) -> Option<&'static str> {
        match self {
            Self::Failed(_) => Some("Повторите запрос или проверьте модель в настройках агента."),
            Self::Canceled => None,
            Self::TimedOut | Self::Truncated => {
                Some("Попросите агента продолжить с места остановки.")
            }
            Self::Filtered => Some("Переформулируйте запрос."),
        }
    }

    pub(crate) fn detail(&self) -> Option<&str> {
        match self {
            Self::Failed(Some(message)) => Some(message),
            _ => None,
        }
    }

    pub(crate) fn class(&self) -> &'static str {
        match self {
            Self::Failed(_) | Self::TimedOut => "turn-issue error",
            Self::Canceled => "turn-issue muted",
            Self::Truncated | Self::Filtered => "turn-issue",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settled_turns_and_finish_reasons_name_the_issue() {
        assert_eq!(
            TurnIssue::from_transcript("AppServer error: 429 rate limit"),
            Some(TurnIssue::Failed(Some("429 rate limit".into())))
        );
        assert_eq!(
            TurnIssue::from_transcript("AppServer canceled: run was canceled"),
            Some(TurnIssue::Canceled)
        );
        assert_eq!(
            TurnIssue::from_transcript("AppServer timeout: turn timed out"),
            Some(TurnIssue::TimedOut)
        );
        assert_eq!(TurnIssue::from_transcript("Обычное сообщение"), None);
        assert_eq!(TurnIssue::from_transcript("AppServer note: что-то"), None);
        assert_eq!(
            TurnIssue::from_finish(&FinishReason::Length),
            Some(Some(TurnIssue::Truncated))
        );
        assert_eq!(TurnIssue::from_finish(&FinishReason::ToolCalls), Some(None));
        assert_eq!(TurnIssue::from_finish(&FinishReason::Unknown), None);
    }
}
