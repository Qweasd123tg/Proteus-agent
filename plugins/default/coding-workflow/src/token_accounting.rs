use proteus_contracts::{
    domain::{TokenUsageCategory, TokenUsageSnapshot, TokenUsageSource, ToolContent},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, ContentPart, MessageRole, TokenUsage,
    },
};
use serde_json::Value;

pub(crate) fn request_token_usage_snapshot(
    request: &CanonicalModelRequest,
    actual: Option<TokenUsage>,
    phase: &str,
) -> TokenUsageSnapshot {
    let mut categories = estimate_request_categories(request);
    let estimated_input_tokens = categories.iter().map(|category| category.tokens).sum();
    if let Some(actual_usage) = actual.as_ref() {
        append_provider_cache_categories(&mut categories, actual_usage);
    }
    let source = if actual.is_some() {
        TokenUsageSource::Mixed
    } else {
        TokenUsageSource::Estimated
    };
    // Порог автокомпакта кладёт в metadata request_from_state по отчёту
    // компактора — берём его, чтобы метка в клиентах совпадала с триггером.
    let compaction_trigger_tokens = request
        .metadata
        .get("compaction_trigger_tokens")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    TokenUsageSnapshot::new(request.model.clone(), estimated_input_tokens, categories)
        .with_phase(phase)
        .with_max_input_tokens(request.limits.max_input_tokens)
        .with_compaction_trigger_tokens(compaction_trigger_tokens)
        .with_actual(actual)
        .with_source(source)
}

fn estimate_request_categories(request: &CanonicalModelRequest) -> Vec<TokenUsageCategory> {
    let mut bytes = RequestCategoryBytes {
        instructions: request
            .instructions
            .iter()
            .map(|instruction| instruction.text.len())
            .sum::<usize>(),
        ..Default::default()
    };
    if !request.instructions.is_empty() {
        bytes.instructions += request.instructions.len() * 8;
    }

    for message in &request.messages {
        bytes.messages += message_envelope_bytes(message);
        for part in &message.parts {
            match part {
                ContentPart::Text { text }
                | ContentPart::ReasoningSummary { text }
                | ContentPart::Reasoning { text, .. } => {
                    bytes.messages += text.len();
                }
                ContentPart::Context { chunk } => {
                    bytes.context += chunk.source.len()
                        + chunk
                            .path
                            .as_ref()
                            .map(|path| path.display().to_string().len())
                            .unwrap_or_default()
                        + chunk.content.len()
                        + chunk.metadata.to_string().len();
                }
                ContentPart::FileRef { path, content } => {
                    bytes.files += path.display().to_string().len()
                        + content.as_deref().unwrap_or_default().len();
                }
                ContentPart::ToolCall { call } => {
                    bytes.tool_calls +=
                        call.id.as_str().len() + call.name.len() + call.args.to_string().len();
                }
                ContentPart::ToolResult { result } => {
                    bytes.tool_results += result.call_id.as_str().len()
                        + result.output.len()
                        + result.error.as_deref().unwrap_or_default().len()
                        + result.metadata.to_string().len()
                        + result
                            .content
                            .iter()
                            .map(tool_content_text_len)
                            .sum::<usize>();
                }
                ContentPart::Patch { patch } => {
                    bytes.patches += patch.content.len();
                }
                _ => {}
            }
        }
    }

    bytes.tool_schemas = request
        .tools
        .iter()
        .map(|tool| {
            serde_json::to_string(tool)
                .map(|json| json.len())
                .unwrap_or(0)
        })
        .sum::<usize>();

    [
        ("instructions", bytes.instructions),
        ("messages", bytes.messages),
        ("context", bytes.context),
        ("tool_calls", bytes.tool_calls),
        ("tool_results", bytes.tool_results),
        ("files", bytes.files),
        ("patches", bytes.patches),
        ("tool_schemas", bytes.tool_schemas),
    ]
    .into_iter()
    .filter_map(|(name, bytes)| {
        let tokens = estimate_tokens_from_bytes(bytes);
        (tokens > 0)
            .then(|| TokenUsageCategory::new(name, tokens).with_source(TokenUsageSource::Estimated))
    })
    .collect()
}

#[derive(Default)]
struct RequestCategoryBytes {
    instructions: usize,
    messages: usize,
    context: usize,
    tool_calls: usize,
    tool_results: usize,
    files: usize,
    patches: usize,
    tool_schemas: usize,
}

fn append_provider_cache_categories(categories: &mut Vec<TokenUsageCategory>, usage: &TokenUsage) {
    [
        ("provider_cache_read", usage.cached_input_tokens),
        ("provider_cache_write", usage.cache_creation_input_tokens),
    ]
    .into_iter()
    .filter_map(|(name, tokens)| {
        tokens
            .filter(|tokens| *tokens > 0)
            .map(|tokens| (name, tokens))
    })
    .for_each(|(name, tokens)| {
        categories
            .push(TokenUsageCategory::new(name, tokens).with_source(TokenUsageSource::Provider));
    });
}

fn message_envelope_bytes(message: &CanonicalMessage) -> usize {
    let role_bytes = match message.role {
        MessageRole::System => "system".len(),
        MessageRole::Developer => "developer".len(),
        MessageRole::User => "user".len(),
        MessageRole::Assistant => "assistant".len(),
        MessageRole::Tool => "tool".len(),
        _ => 0,
    };
    role_bytes
        + message.name.as_deref().map(str::len).unwrap_or_default()
        + message
            .tool_call_id
            .as_ref()
            .map(|id| id.as_str().len())
            .unwrap_or_default()
        + message.metadata.to_string().len()
        + 4
}

fn estimate_tokens_from_bytes(bytes: usize) -> u32 {
    if bytes == 0 {
        0
    } else {
        (bytes / 4).max(1) as u32
    }
}

fn tool_content_text_len(content: &ToolContent) -> usize {
    match content {
        ToolContent::Text { text } => text.len(),
        ToolContent::Json { value } => value.to_string().len(),
        ToolContent::Image { data, .. } | ToolContent::Binary { data, .. } => data.len(),
        _ => 0,
    }
}

/// Реальный usage последнего model-ответа — точка отсчёта для оценки давления
/// на контекст в следующем compaction-чеке (как inline auto-compact в Codex).
pub(crate) struct LastModelUsage {
    pub(crate) usage: TokenUsage,
    /// Сколько сообщений model_messages покрывал этот usage (включая
    /// assistant-ответ). Всё, что добавлено позже, оценивается по chars/4.
    pub(crate) message_count: usize,
}

/// Оценка токенов истории для триггера компактора: если есть реальный usage
/// прошлого запроса, берём его (input+output) плюс chars/4-дельту новых
/// сообщений; chars/4 по всей истории остаётся нижней границей и fallback-ом.
pub(crate) fn effective_token_estimate(
    messages: &[CanonicalMessage],
    last_usage: Option<&LastModelUsage>,
) -> Option<u32> {
    let char_estimate = estimate_message_tokens(messages);
    let Some(last) = last_usage else {
        return char_estimate;
    };
    if last.message_count > messages.len() {
        // История сжалась после замера (компакция) — usage устарел.
        return char_estimate;
    }
    let known = last
        .usage
        .input_tokens
        .saturating_add(last.usage.output_tokens);
    let delta = estimate_message_tokens(&messages[last.message_count..]).unwrap_or(0);
    let usage_based = known.saturating_add(delta);
    Some(char_estimate.unwrap_or(0).max(usage_based))
}

pub(crate) fn estimate_message_tokens(messages: &[CanonicalMessage]) -> Option<u32> {
    let bytes = messages
        .iter()
        .flat_map(|message| &message.parts)
        .map(part_text_len)
        .sum::<usize>();
    Some((bytes / 4 + messages.len()).max(1) as u32)
}

fn part_text_len(part: &ContentPart) -> usize {
    match part {
        ContentPart::Text { text } => text.len(),
        ContentPart::Context { chunk } => chunk.content.len(),
        ContentPart::FileRef { content, .. } => content.as_deref().unwrap_or_default().len(),
        ContentPart::ToolCall { call } => call.name.len() + call.args.to_string().len(),
        ContentPart::ToolResult { result } => {
            result.output.len()
                + result.error.as_deref().unwrap_or_default().len()
                + result.metadata.to_string().len()
        }
        ContentPart::Patch { patch } => patch.content.len(),
        ContentPart::ReasoningSummary { text } | ContentPart::Reasoning { text, .. } => text.len(),
        _ => 0,
    }
}

#[cfg(test)]
mod effective_estimate_tests {
    use super::*;

    fn text_message(text: &str) -> CanonicalMessage {
        CanonicalMessage::text(MessageRole::User, text.to_owned())
    }

    #[test]
    fn without_usage_falls_back_to_char_estimate() {
        let messages = vec![text_message(&"x".repeat(400))];
        assert_eq!(
            effective_token_estimate(&messages, None),
            estimate_message_tokens(&messages)
        );
    }

    #[test]
    fn usage_dominates_char_estimate_for_covered_history() {
        // Реальный usage сильно выше chars/4 (thinking, schemas, плотный код).
        let messages = vec![text_message("short"), text_message("reply")];
        let last = LastModelUsage {
            usage: TokenUsage::new(50_000, 2_000),
            message_count: 2,
        };

        let estimate = effective_token_estimate(&messages, Some(&last)).expect("estimate");

        assert!(estimate >= 52_000, "{estimate}");
    }

    #[test]
    fn new_messages_after_usage_add_char_delta() {
        let mut messages = vec![text_message("short"), text_message("reply")];
        let last = LastModelUsage {
            usage: TokenUsage::new(10_000, 500),
            message_count: 2,
        };
        let base = effective_token_estimate(&messages, Some(&last)).expect("base");

        messages.push(text_message(&"y".repeat(4_000)));
        let grown = effective_token_estimate(&messages, Some(&last)).expect("grown");

        assert!(grown >= base + 900, "base={base}, grown={grown}");
    }

    #[test]
    fn stale_usage_after_history_shrink_is_ignored() {
        let messages = vec![text_message("short")];
        let last = LastModelUsage {
            usage: TokenUsage::new(90_000, 1_000),
            message_count: 5,
        };

        assert_eq!(
            effective_token_estimate(&messages, Some(&last)),
            estimate_message_tokens(&messages)
        );
    }
}
