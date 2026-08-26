//! Helpers for process-peer terminal results and usage accounting.

use crate::{contracts::SubagentStatus, model_standard::TokenUsage};

/// Суммирует usage нескольких child events в один optional accumulator.
pub(super) fn accumulate_usage(total: &mut Option<TokenUsage>, usage: Option<&TokenUsage>) {
    let Some(usage) = usage else {
        return;
    };
    match total {
        None => *total = Some(usage.clone()),
        Some(total) => total.accumulate(usage),
    }
}

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
pub(super) fn status_label(status: SubagentStatus) -> String {
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
    fn usage_accumulation_sums_option_fields() {
        let mut total = None;
        accumulate_usage(&mut total, Some(&TokenUsage::new(10, 2)));
        accumulate_usage(
            &mut total,
            Some(
                &TokenUsage::new(5, 3)
                    .with_cached_input_tokens(Some(4))
                    .with_reasoning_output_tokens(Some(7)),
            ),
        );
        accumulate_usage(&mut total, None);

        let total = total.expect("usage accumulated");
        assert_eq!(total.input_tokens, 15);
        assert_eq!(total.output_tokens, 5);
        assert_eq!(total.cached_input_tokens, Some(4));
        assert_eq!(total.cache_creation_input_tokens, None);
        assert_eq!(total.reasoning_output_tokens, Some(7));
    }

    #[test]
    fn status_label_is_snake_case() {
        assert_eq!(status_label(SubagentStatus::Completed), "completed");
        assert_eq!(status_label(SubagentStatus::TimedOut), "timed_out");
        assert_eq!(status_label(SubagentStatus::Cancelled), "cancelled");
        assert_eq!(
            status_label(SubagentStatus::TokenBudgetExceeded),
            "token_budget_exceeded"
        );
    }
}
