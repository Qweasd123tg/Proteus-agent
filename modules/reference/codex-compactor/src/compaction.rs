use proteus_contracts::{
    contracts::{CompactionInput, CompactionOutput},
    plugin::PluginCompactorHostMut,
};
use serde_json::json;

use crate::{
    MODULE_ID,
    budget::{estimate_messages_tokens, resolve_trigger_tokens, user_message_budget_tokens},
    history::{
        collect_user_messages, replacement_messages, select_recent_user_messages, split_history,
    },
    summary::try_model_summary,
};

pub(crate) fn compact(
    input: CompactionInput,
    host: &mut PluginCompactorHostMut<'_>,
) -> Result<CompactionOutput, String> {
    if input.messages.is_empty() {
        return Ok(CompactionOutput::unchanged(input.messages));
    }

    let token_estimate = input
        .token_estimate
        .unwrap_or_else(|| estimate_messages_tokens(&input.messages));
    let trigger_tokens = resolve_trigger_tokens(&input);
    if token_estimate <= trigger_tokens {
        return Ok(unchanged_with_metadata(
            input.messages,
            token_estimate,
            trigger_tokens,
            "below_trigger_threshold",
        ));
    }

    let history = split_history(&input.messages);
    if history.compactable_history.is_empty() {
        return Ok(unchanged_with_metadata(
            input.messages,
            token_estimate,
            trigger_tokens,
            "no_persistent_history_to_compact",
        ));
    }

    let user_messages = collect_user_messages(&history.compactable_history);
    let preserved_user_messages =
        select_recent_user_messages(&user_messages, user_message_budget_tokens());
    let summary = try_model_summary(&input, &history.summary_history, host)?;
    let replacement = replacement_messages(
        &history.ephemeral_context,
        &preserved_user_messages,
        &summary,
    );
    let output_token_estimate = estimate_messages_tokens(&replacement);
    if output_token_estimate >= token_estimate {
        return Err(format!(
            "codex compaction replacement would not reduce tokens: input={token_estimate}, output={output_token_estimate}"
        ));
    }

    let output_messages = replacement.len();
    let mut output = CompactionOutput::changed(replacement, Some(summary));
    output.token_estimate = Some(output_token_estimate);
    output.metadata = json!({
        "compactor": MODULE_ID,
        "summary_source": "model",
        "input_messages": input.messages.len(),
        "output_messages": output_messages,
        "original_token_estimate": token_estimate,
        "output_token_estimate": output_token_estimate,
        "trigger_tokens": trigger_tokens,
        "compacted_messages": history.compactable_history.len(),
        "preserved_user_messages": preserved_user_messages.len(),
        "ephemeral_context_messages": history.ephemeral_context.len(),
    });
    Ok(output)
}

fn unchanged_with_metadata(
    messages: Vec<proteus_contracts::model_standard::CanonicalMessage>,
    token_estimate: u32,
    trigger_tokens: u32,
    reason: &str,
) -> CompactionOutput {
    let mut output = CompactionOutput::unchanged(messages);
    output.token_estimate = Some(token_estimate);
    output.metadata = json!({
        "compactor": MODULE_ID,
        "skipped_reason": reason,
        "original_token_estimate": token_estimate,
        "trigger_tokens": trigger_tokens,
    });
    output
}
