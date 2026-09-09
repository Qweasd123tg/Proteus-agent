use proteus_contracts::{
    contracts::{CompactionInput, CompactionOutput},
    process_module::{CompactorModuleHostMut, ProcessModuleError},
};
use serde_json::json;

use crate::{
    budget::{estimate_messages_tokens, resolve_trigger_tokens, user_message_budget_tokens},
    history::{
        collect_user_messages, replacement_messages, select_recent_user_messages, split_history,
    },
    recovery::complete_summary_with_recovery,
};

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
        &preserved_user_messages.messages,
        &summary,
    );
    let output_token_estimate = estimate_messages_tokens(&replacement);

    let mut output = CompactionOutput::changed(replacement, Some(summary));
    output.user_message_replacements = preserved_user_messages.replacements;
    output.token_estimate = Some(output_token_estimate);
    output.original_token_estimate = Some(token_estimate);
    output.trigger_tokens = Some(trigger_tokens);
    output.summary_source = Some("model".to_owned());
    output.metadata = json!({
        "compacted_messages": history.compactable_history.len(),
        "preserved_user_messages": preserved_user_messages.messages.len(),
        "ephemeral_context_messages": history.ephemeral_context.len(),
    });
    Ok(output)
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
