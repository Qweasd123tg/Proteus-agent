use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    contracts::CompactionInput,
    domain::{CacheHints, ToolChoice},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        InstructionBlock, InstructionKind, MessageRole,
    },
    plugin::PluginCompactorHostMut,
};
use serde_json::json;

use crate::{
    MODULE_ID,
    budget::{summary_budget_tokens, truncate_to_tokens},
    history::message_text,
};

const SUMMARY_SYSTEM_INSTRUCTIONS: &str = "You are compressing earlier conversation history for a coding agent handoff. Summarize only; do not solve the user's task.";
pub(crate) const SUMMARY_PREFIX: &str = "Another language model started to solve this problem and produced a summary of its thinking process. You also have access to the state of the tools that were used by that language model. Use this to build on the work that has already been done and avoid duplicating work. Here is the summary produced by the other language model, use the information in this summary to assist with your own analysis:";

pub(crate) fn try_model_summary(
    input: &CompactionInput,
    summary_history: &[CanonicalMessage],
    host: &mut PluginCompactorHostMut<'_>,
) -> Result<String, String> {
    ensure_not_cancelled(host)?;
    let summary_budget = summary_budget_tokens()?;
    let request = model_summary_request(input, summary_history, summary_budget);
    let request_json = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    let response_json = match host.complete_model_json(RString::from(request_json)) {
        RResult::ROk(json) => json,
        RResult::RErr(error) => return Err(error.message.into_string()),
    };
    ensure_not_cancelled(host)?;
    let response: CanonicalModelResponse =
        serde_json::from_str(response_json.as_str()).map_err(|error| {
            format!("codex compaction model returned invalid response JSON: {error}")
        })?;
    let text = validate_summary_response(&response)?;
    Ok(summary_with_prefix(&text, summary_budget))
}

fn validate_summary_response(response: &CanonicalModelResponse) -> Result<String, String> {
    if response.message.role != MessageRole::Assistant {
        return Err("codex compaction model summary must use assistant role".to_owned());
    }
    if response.finish_reason != FinishReason::Stop {
        return Err(format!(
            "codex compaction model summary must finish with Stop, got {:?}",
            response.finish_reason
        ));
    }
    if !response.tool_calls.is_empty()
        || response
            .message
            .parts
            .iter()
            .any(|part| matches!(part, ContentPart::ToolCall { .. }))
    {
        return Err("codex compaction model summary must not request tools".to_owned());
    }
    let Some(text) = message_text(&response.message) else {
        return Err("codex compaction model returned no summary text".to_owned());
    };
    let text = text.trim();
    if text.is_empty() {
        return Err("codex compaction model returned empty summary text".to_owned());
    }
    Ok(text.to_owned())
}

fn model_summary_request(
    input: &CompactionInput,
    summary_history: &[CanonicalMessage],
    summary_budget: u32,
) -> CanonicalModelRequest {
    let mut messages = summary_history.to_vec();
    messages.push(CanonicalMessage::text(
        MessageRole::User,
        model_summary_prompt(input, summary_history.len()),
    ));
    let mut request = CanonicalModelRequest::new(input.model_ref.clone(), messages)
        .with_instructions(vec![InstructionBlock::new(
            InstructionKind::System,
            SUMMARY_SYSTEM_INSTRUCTIONS,
            100,
        )])
        .with_tool_choice(ToolChoice::None)
        .with_cache(CacheHints::new(true, false))
        .with_metadata(json!({
            "compactor": MODULE_ID,
            "phase": "history_compaction",
            "prompt_cache_key": prompt_cache_key(input),
            "suppress_stream_deltas": true,
        }));
    request.limits.max_output_tokens = Some(summary_budget);
    request
}

fn prompt_cache_key(input: &CompactionInput) -> String {
    let workspace_hash = stable_hash64(input.task.cwd.to_string_lossy().as_bytes());
    let request_shape = format!(
        "{}\0{}\0{}\0{}",
        input.model_ref.provider,
        input.model_ref.model,
        SUMMARY_SYSTEM_INSTRUCTIONS,
        SUMMARY_PREFIX,
    );
    let request_shape_hash = stable_hash64(request_shape.as_bytes());

    // OpenAI accepts at most 64 characters for `prompt_cache_key`. Hash all
    // unbounded components instead of truncating provider/model independently,
    // which could still produce a key well above the provider limit.
    format!("proteus:compact:{workspace_hash:016x}:{request_shape_hash:016x}")
}

fn stable_hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn model_summary_prompt(input: &CompactionInput, compacted_messages: usize) -> String {
    let mut prompt = String::new();
    prompt.push_str("You are performing a CONTEXT CHECKPOINT COMPACTION.\n\n");
    prompt.push_str("Summarize the conversation and tool state so another model can continue the same coding task without rereading all compacted messages.\n\n");
    prompt.push_str("Return only the handoff summary body. Do not include the standard Codex prefix; the runtime will add it. Do not answer the current user task.\n\n");
    prompt.push_str("Preserve:\n");
    prompt.push_str("- current user goal and latest requested behavior\n");
    prompt.push_str("- files changed or inspected, commands run, and important results\n");
    prompt.push_str("- architectural decisions, constraints, and invariants\n");
    prompt.push_str("- unresolved blockers, risks, and exact next steps\n");
    prompt.push_str(
        "- exact paths, module ids, config keys, error strings, and test names when relevant\n\n",
    );
    prompt.push_str("Current task:\n");
    prompt.push_str(&input.task.text);
    prompt.push_str("\n\n");
    prompt.push_str(&format!("Compacted messages: {compacted_messages}\n"));
    if let Some(reason) = input.reason.as_deref().filter(|reason| !reason.is_empty()) {
        prompt.push_str("Compaction reason: ");
        prompt.push_str(reason);
        prompt.push('\n');
    }
    prompt
}

fn summary_with_prefix(text: &str, summary_budget: u32) -> String {
    let text = text.trim();
    let summary = if text.starts_with(SUMMARY_PREFIX) {
        text.to_owned()
    } else {
        format!("{SUMMARY_PREFIX}\n\n{text}")
    };
    truncate_to_tokens(&summary, summary_budget as usize)
}

fn ensure_not_cancelled(host: &mut PluginCompactorHostMut<'_>) -> Result<(), String> {
    match host.is_cancelled() {
        RResult::ROk(false) => Ok(()),
        RResult::ROk(true) => Err("turn canceled by client".to_owned()),
        RResult::RErr(error) => Err(error.message.into_string()),
    }
}

#[cfg(test)]
pub(crate) fn prompt_cache_key_for_test(input: &CompactionInput) -> String {
    prompt_cache_key(input)
}

#[cfg(test)]
pub(crate) fn validate_summary_response_for_test(
    response: &CanonicalModelResponse,
) -> Result<String, String> {
    validate_summary_response(response)
}
