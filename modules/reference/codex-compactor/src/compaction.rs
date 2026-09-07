use proteus_contracts::{
    contracts::{CompactionInput, CompactionOutput},
    model_standard::{CanonicalMessage, ContentPart, ModelFailureKind},
    process_module::{CompactorModuleHostMut, ProcessModuleError},
};
use rand::Rng;
use serde_json::json;
use std::time::Duration;

use crate::{
    budget::{estimate_messages_tokens, resolve_trigger_tokens, user_message_budget_tokens},
    history::{
        collect_user_messages, replacement_messages, select_recent_user_messages, split_history,
    },
    summary::{ensure_not_cancelled, try_model_summary},
};

const MAX_MODEL_RETRIES: u32 = 5;

pub(crate) fn compact(
    input: CompactionInput,
    host: &mut CompactorModuleHostMut<'_>,
) -> Result<CompactionOutput, ProcessModuleError> {
    if input.request.messages.is_empty() {
        return Ok(CompactionOutput::unchanged(input.request.messages));
    }

    let token_estimate = input
        .token_estimate
        .unwrap_or_else(|| estimate_messages_tokens(&input.request.messages));
    let Some(trigger_tokens) = resolve_trigger_tokens(&input).map_err(ProcessModuleError::new)?
    else {
        return Ok(unchanged_without_trigger(
            input.request.messages,
            token_estimate,
        ));
    };
    if token_estimate < trigger_tokens {
        return Ok(unchanged_with_diagnostics(
            input.request.messages,
            token_estimate,
            trigger_tokens,
            "below_trigger_threshold",
        ));
    }

    let history = split_history(&input.request.messages);
    if history.compactable_history.is_empty() {
        return Ok(unchanged_with_diagnostics(
            input.request.messages,
            token_estimate,
            trigger_tokens,
            "no_persistent_history_to_compact",
        ));
    }

    let user_messages = collect_user_messages(&history.compactable_history);
    let preserved_user_messages =
        select_recent_user_messages(&user_messages, user_message_budget_tokens());
    let summary = complete_summary_with_recovery(&input, history.summary_history, host)?;
    let replacement = replacement_messages(
        &history.ephemeral_context,
        &preserved_user_messages,
        &summary,
    );
    let output_token_estimate = estimate_messages_tokens(&replacement);

    let mut output = CompactionOutput::changed(replacement, Some(summary));
    output.token_estimate = Some(output_token_estimate);
    output.original_token_estimate = Some(token_estimate);
    output.trigger_tokens = Some(trigger_tokens);
    output.summary_source = Some("model".to_owned());
    output.metadata = json!({
        "compacted_messages": history.compactable_history.len(),
        "preserved_user_messages": preserved_user_messages.len(),
        "ephemeral_context_messages": history.ephemeral_context.len(),
    });
    Ok(output)
}

fn complete_summary_with_recovery(
    input: &CompactionInput,
    mut history: Vec<CanonicalMessage>,
    host: &mut CompactorModuleHostMut<'_>,
) -> Result<String, ProcessModuleError> {
    let mut retries = 0;
    loop {
        match try_model_summary(input, &history, host) {
            Ok(summary) => return Ok(summary),
            Err(error) => match error.model_failure.as_ref().map(|failure| failure.kind) {
                Some(ModelFailureKind::Interrupted | ModelFailureKind::SessionBudgetExceeded) => {
                    return Err(error);
                }
                Some(ModelFailureKind::ContextWindowExceeded) => {
                    if history.is_empty() {
                        return Err(error);
                    }
                    remove_first_message_and_counterpart(&mut history);
                    retries = 0;
                }
                Some(ModelFailureKind::Other) | None if retries < MAX_MODEL_RETRIES => {
                    retries += 1;
                    wait_for_retry(host, retry_delay(retries))?;
                }
                Some(ModelFailureKind::Other) | Some(_) | None => return Err(error),
            },
        }
    }
}

// Pinned Codex uses a 200ms exponential reconnect backoff (factor 2) plus a
// random multiplier in [0.9, 1.1).
fn retry_delay(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(63);
    let base = 200u64.saturating_mul(1u64 << exponent);
    let jitter = rand::rng().random_range(0.9..1.1);
    Duration::from_millis((base as f64 * jitter) as u64)
}

fn wait_for_retry(
    host: &mut CompactorModuleHostMut<'_>,
    delay: Duration,
) -> Result<(), ProcessModuleError> {
    let mut remaining = delay;
    let slice = Duration::from_millis(20);
    while !remaining.is_zero() {
        ensure_not_cancelled(host)?;
        let current = remaining.min(slice);
        std::thread::sleep(current);
        remaining = remaining.saturating_sub(current);
    }
    ensure_not_cancelled(host)
}

/// Equivalent to Codex `ContextManager::remove_first_item` for the canonical
/// message form: dropping a call also drops its result, and vice versa. A
/// canonical message can group a batch, so counterparts are removed at part
/// granularity to keep unrelated calls in the same assistant item intact.
fn remove_first_message_and_counterpart(history: &mut Vec<CanonicalMessage>) {
    let removed = history.remove(0);
    let mut call_ids = removed.tool_call_id.into_iter().collect::<Vec<_>>();
    call_ids.extend(removed.parts.iter().filter_map(part_call_id));
    if call_ids.is_empty() {
        return;
    }
    history.retain_mut(|message| {
        let standalone_counterpart = message
            .tool_call_id
            .as_ref()
            .is_some_and(|id| call_ids.iter().any(|removed_id| removed_id == id));
        if standalone_counterpart {
            return false;
        }
        message.parts.retain(|part| {
            !part_call_id(part)
                .as_ref()
                .is_some_and(|id| call_ids.iter().any(|removed_id| removed_id == id))
        });
        !message.parts.is_empty()
    });
}

fn part_call_id(part: &proteus_contracts::model_standard::CanonicalPart) -> Option<String> {
    match &part.payload {
        ContentPart::ToolCall { call } => Some(call.id.clone()),
        ContentPart::ToolResult { result } => Some(result.call_id.clone()),
        _ => None,
    }
}

fn unchanged_without_trigger(
    messages: Vec<proteus_contracts::model_standard::CanonicalMessage>,
    token_estimate: u32,
) -> CompactionOutput {
    let mut output = CompactionOutput::unchanged(messages);
    output.token_estimate = Some(token_estimate);
    output.original_token_estimate = Some(token_estimate);
    output.skipped_reason = Some("no_trigger_threshold".to_owned());
    output
}

fn unchanged_with_diagnostics(
    messages: Vec<proteus_contracts::model_standard::CanonicalMessage>,
    token_estimate: u32,
    trigger_tokens: u32,
    reason: &str,
) -> CompactionOutput {
    let mut output = CompactionOutput::unchanged(messages);
    output.token_estimate = Some(token_estimate);
    output.original_token_estimate = Some(token_estimate);
    output.trigger_tokens = Some(trigger_tokens);
    output.skipped_reason = Some(reason.to_owned());
    output.metadata = serde_json::Value::Null;
    output
}
