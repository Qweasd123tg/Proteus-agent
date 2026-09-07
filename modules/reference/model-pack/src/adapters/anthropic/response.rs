use anyhow::{Result, anyhow};
use serde_json::Value;

use super::sanitize::sanitize_provider_text;
use crate::{
    domain::ToolCall,
    model_standard::{
        CanonicalMessage, CanonicalModelResponse, ContentPart, FinishReason, MessageRole,
        TokenUsage,
    },
};

pub(super) fn from_anthropic_response(response: Value) -> Result<CanonicalModelResponse> {
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Err(anyhow!("Anthropic API error: {error}"));
    }

    let mut parts = Vec::new();
    let mut tool_calls = Vec::new();
    let finish_reason = match response.get("stop_reason").and_then(Value::as_str) {
        Some("tool_use") => FinishReason::ToolCalls,
        Some("end_turn") | Some("stop_sequence") => FinishReason::Stop,
        Some("max_tokens") => FinishReason::Length,
        Some(_) => FinishReason::Unknown,
        None => FinishReason::Unknown,
    };
    let accept_tool_calls = finish_reason == FinishReason::ToolCalls;

    for item in response
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Anthropic response did not contain content array"))?
    {
        match item.get("type").and_then(Value::as_str) {
            Some("thinking") => {
                if let Some(text) = item.get("thinking").and_then(Value::as_str) {
                    let signature = item
                        .get("signature")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    if !text.trim().is_empty() || signature.is_some() {
                        parts.push(ContentPart::Reasoning {
                            text: text.to_owned(),
                            signature,
                        });
                    }
                }
            }
            Some("text") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    let text = sanitize_provider_text(text);
                    if !text.is_empty() {
                        parts.push(ContentPart::Text { text });
                    }
                }
            }
            Some("tool_use") if accept_tool_calls => {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("tool_use missing id"))?
                    .to_owned();
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("tool_use missing name"))?
                    .to_owned();
                let args = item.get("input").cloned().unwrap_or(Value::Null);
                let call = ToolCall::new(id, name, args);
                parts.push(ContentPart::ToolCall { call: call.clone() });
                tool_calls.push(call);
            }
            Some("tool_use") => {}
            _ => {}
        }
    }

    let message = CanonicalMessage::new(MessageRole::Assistant, parts);
    let usage = parse_usage(&response);
    let mut resp = CanonicalModelResponse::new(message, tool_calls, finish_reason);
    if let Some(u) = usage {
        resp = resp.with_usage(u);
    }
    resp = resp.with_provider_metadata(response);
    Ok(resp)
}

fn parse_usage(response: &Value) -> Option<TokenUsage> {
    let usage = response.get("usage")?;
    let input_tokens = usage.get("input_tokens")?.as_u64()? as u32;
    let output_tokens = usage.get("output_tokens")?.as_u64()? as u32;
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64)
        .map(|tokens| tokens as u32);
    let cache_read = usage
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .map(|tokens| tokens as u32);
    Some(
        TokenUsage::new(input_tokens, output_tokens)
            .with_cache_creation_input_tokens(cache_creation)
            .with_cached_input_tokens(cache_read),
    )
}
