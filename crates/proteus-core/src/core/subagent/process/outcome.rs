//! Helpers for process-peer terminal results.

use crate::contracts::AgentLifecycleStatus;

/// Обрезает summary по границе char до указанного byte-limit.
pub(super) fn truncate_summary(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// Canonical snake_case label для `Event::SubagentFinished`.
pub(super) fn status_label(status: AgentLifecycleStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{status:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_truncation_respects_char_boundaries() {
        let truncated = truncate_summary("й".repeat(4), 5);
        assert_eq!(truncated, "йй");
        assert!(truncated.len() <= 5);
        assert!(truncated.is_char_boundary(truncated.len()));
        assert_eq!(truncate_summary("abc".to_owned(), 5), "abc");
    }

    #[test]
    fn status_label_is_snake_case() {
        assert_eq!(status_label(AgentLifecycleStatus::Completed), "completed");
        assert_eq!(status_label(AgentLifecycleStatus::TimedOut), "timed_out");
        assert_eq!(status_label(AgentLifecycleStatus::Cancelled), "cancelled");
    }
}
