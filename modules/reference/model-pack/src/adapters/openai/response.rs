use std::collections::HashMap;

use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::model_standard::{
    CanonicalMessage, CanonicalModelResponse, ContentPart, FinishReason, MessageRole, TokenUsage,
};

#[path = "response/item.rs"]
mod item;
pub(super) use item::{parse_message_phase, parse_output_item};

pub(super) fn from_openai_response(response: Value) -> Result<CanonicalModelResponse> {
    from_openai_response_with_ids(response, &HashMap::new())
}

pub(super) fn item_key(item: &Value, index: usize) -> String {
    item.get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("output:{index}"))
}

pub(super) fn from_openai_response_with_ids(
    response: Value,
    ids: &HashMap<String, crate::domain::MessageId>,
) -> Result<CanonicalModelResponse> {
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Err(anyhow!("OpenAI API error: {error}"));
    }
    if response.get("status").and_then(Value::as_str) == Some("incomplete") {
        let reason = response
            .get("incomplete_details")
            .and_then(|details| details.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(anyhow!("Incomplete response returned, reason: {reason}"));
    }

    let mut messages = Vec::new();
    let mut tool_calls = Vec::new();

    for (index, item) in response
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("OpenAI response did not contain output array"))?
        .iter()
        .enumerate()
    {
        if let Some(mut message) = parse_output_item(item)? {
            if let Some(id) = ids.get(&item_key(item, index)) {
                message.id = *id;
            }
            tool_calls.extend(message.parts.iter().filter_map(|part| match &part.payload {
                ContentPart::ToolCall { call } => Some(call.clone()),
                _ => None,
            }));
            messages.push(message);
        }
    }

    let finish_reason = if tool_calls.is_empty() {
        FinishReason::Stop
    } else {
        FinishReason::ToolCalls
    };
    if messages.is_empty() {
        messages.push(CanonicalMessage::new(MessageRole::Assistant, Vec::new()));
    }
    let usage = parse_usage(&response);
    let mut resp = CanonicalModelResponse::from_messages(messages, tool_calls, finish_reason);
    if let Some(u) = usage {
        resp = resp.with_usage(u);
    }
    if let Some(end_turn) = response.get("end_turn").and_then(Value::as_bool) {
        resp = resp.with_end_turn(end_turn);
    }
    Ok(resp.with_provider_metadata(response))
}

fn parse_usage(response: &Value) -> Option<TokenUsage> {
    let usage = response.get("usage")?;
    let input_tokens = usage.get("input_tokens")?.as_u64()? as u32;
    let output_tokens = usage.get("output_tokens")?.as_u64()? as u32;
    let cached_input_tokens = usage
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64)
        .map(|tokens| tokens as u32);
    let reasoning_output_tokens = usage
        .get("output_tokens_details")
        .and_then(|details| details.get("reasoning_tokens"))
        .and_then(Value::as_u64)
        .map(|tokens| tokens as u32);

    Some(
        TokenUsage::new(input_tokens, output_tokens)
            .with_cached_input_tokens(cached_input_tokens)
            .with_reasoning_output_tokens(reasoning_output_tokens),
    )
}
