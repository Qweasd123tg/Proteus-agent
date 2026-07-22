use proteus_contracts::{
    domain::CONTEXT_MESSAGE_NAME,
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};
use serde_json::json;

use crate::{MODULE_ID, budget::truncate_to_tokens, summary::SUMMARY_PREFIX};

pub(crate) struct HistoryParts {
    /// Canonical workspace context is request-scoped. It is re-injected into
    /// the compacted model request, while `coding-workflow` filters it before
    /// persisting replacement history.
    pub(crate) ephemeral_context: Vec<CanonicalMessage>,
    /// Messages shown to the summary model, including canonical context.
    pub(crate) summary_history: Vec<CanonicalMessage>,
    /// Durable conversation items from which bounded real user messages are
    /// selected for replacement history.
    pub(crate) compactable_history: Vec<CanonicalMessage>,
}

pub(crate) fn split_history(messages: &[CanonicalMessage]) -> HistoryParts {
    let mut ephemeral_context = Vec::new();
    let mut summary_history = Vec::new();
    let mut compactable_history = Vec::new();

    for message in messages {
        if is_structured_ephemeral_context_message(message) {
            ephemeral_context.push(message.clone());
            summary_history.push(message.clone());
        } else {
            summary_history.push(message.clone());
            compactable_history.push(message.clone());
        }
    }

    HistoryParts {
        ephemeral_context,
        summary_history,
        compactable_history,
    }
}

pub(crate) fn collect_user_messages(messages: &[CanonicalMessage]) -> Vec<CanonicalMessage> {
    messages
        .iter()
        .filter(|message| is_real_user_message(message))
        .cloned()
        .collect()
}

fn is_real_user_message(message: &CanonicalMessage) -> bool {
    if message.role != MessageRole::User || is_structured_ephemeral_context_message(message) {
        return false;
    }
    let Some(text) = message_text(message) else {
        return false;
    };
    !is_generated_user_message(text.trim_start())
}

fn is_generated_user_message(text: &str) -> bool {
    text.starts_with("<turn_aborted>") || text.starts_with(SUMMARY_PREFIX)
}

fn is_structured_ephemeral_context_message(message: &CanonicalMessage) -> bool {
    message.name.as_deref() == Some(CONTEXT_MESSAGE_NAME)
        || (!message.parts.is_empty()
            && message
                .parts
                .iter()
                .all(|part| matches!(part, ContentPart::Context { .. })))
}

pub(crate) fn select_recent_user_messages(
    messages: &[CanonicalMessage],
    budget_tokens: usize,
) -> Vec<CanonicalMessage> {
    if budget_tokens == 0 {
        return Vec::new();
    }

    let mut selected = Vec::new();
    let mut remaining = budget_tokens;
    for message in messages.iter().rev() {
        if remaining == 0 {
            break;
        }
        let Some(text) = message_text(message) else {
            continue;
        };
        let tokens = crate::budget::estimate_text_tokens(&text);
        if tokens <= remaining {
            selected.push(message.clone());
            remaining = remaining.saturating_sub(tokens);
        } else {
            let mut truncated = message.clone();
            truncated.parts = vec![ContentPart::Text {
                text: truncate_to_tokens(&text, remaining),
            }];
            selected.push(truncated);
            break;
        }
    }
    selected.reverse();
    selected
}

pub(crate) fn replacement_messages(
    ephemeral_context: &[CanonicalMessage],
    preserved_user_messages: &[CanonicalMessage],
    summary: &str,
) -> Vec<CanonicalMessage> {
    let mut replacement = preserved_user_messages.to_vec();

    // Upstream Codex keeps the compaction summary last. For mid-turn
    // compaction it re-injects canonical initial context immediately before
    // the last real user message; if no user survives the budget, context is
    // placed before the summary instead.
    let context_insertion_index = replacement.len().saturating_sub(1);
    replacement.splice(
        context_insertion_index..context_insertion_index,
        ephemeral_context.iter().cloned(),
    );
    replacement.push(
        CanonicalMessage::text(MessageRole::User, summary.to_owned()).with_metadata(json!({
            "compactor": MODULE_ID,
            "summary": true,
        })),
    );
    replacement
}

pub(crate) fn message_text(message: &CanonicalMessage) -> Option<String> {
    let pieces = message
        .parts
        .iter()
        .filter_map(part_text)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join("\n"))
    }
}

fn part_text(part: &ContentPart) -> Option<String> {
    match part {
        ContentPart::Text { text } => Some(text.clone()),
        ContentPart::Context { chunk } => Some(chunk.content.clone()),
        ContentPart::FileRef { path, content } => content
            .as_ref()
            .map(|content| format!("{}:\n{content}", path.display())),
        ContentPart::ToolCall { call } => Some(format!("tool call {} {}", call.name, call.args)),
        ContentPart::ToolResult { result } => Some(result.text_or_status()),
        ContentPart::Patch { patch } => Some(patch.content.clone()),
        ContentPart::ReasoningSummary { text } => Some(text.clone()),
        ContentPart::Reasoning { .. } => None,
        ContentPart::HostedToolActivity { activity } => Some(format!(
            "provider-hosted tool {}: {}",
            activity.kind().as_str(),
            serde_json::to_string(activity).unwrap_or_else(|_| "<unavailable>".to_owned())
        )),
        // The cited assistant text is already present in the adjacent Text
        // part; annotations do not add model-visible prose to summarize.
        ContentPart::Citation { .. } => None,
        _ => None,
    }
}
