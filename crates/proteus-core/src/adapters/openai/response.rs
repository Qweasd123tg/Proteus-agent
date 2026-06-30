use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::{
    domain::{ToolCall, ToolCallSurface},
    model_standard::{
        CanonicalMessage, CanonicalModelResponse, ContentPart, FinishReason, MessageRole,
        TokenUsage,
    },
};

pub(super) fn from_openai_response(response: Value) -> Result<CanonicalModelResponse> {
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Err(anyhow!("OpenAI API error: {error}"));
    }

    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    let length_limited = is_length_limited_response(&response);

    for item in response
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("OpenAI response did not contain output array"))?
    {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for content_item in content {
                        if content_item.get("type").and_then(Value::as_str) == Some("output_text")
                            && let Some(text) = content_item.get("text").and_then(Value::as_str)
                        {
                            text_parts.push(text.to_owned());
                        }
                    }
                }
            }
            Some("function_call") if !length_limited => {
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("id").and_then(Value::as_str))
                    .ok_or_else(|| anyhow!("function_call missing call_id"))?
                    .to_owned();
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("function_call missing name"))?
                    .to_owned();
                let args = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .map(serde_json::from_str)
                    .transpose()?
                    .unwrap_or(Value::Null);
                tool_calls.push(ToolCall::new(call_id, name, args));
            }
            Some("custom_tool_call") if !length_limited => {
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("id").and_then(Value::as_str))
                    .ok_or_else(|| anyhow!("custom_tool_call missing call_id"))?
                    .to_owned();
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("custom_tool_call missing name"))?
                    .to_owned();
                let input = item
                    .get("input")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("custom_tool_call missing input"))?
                    .to_owned();
                tool_calls.push(
                    ToolCall::new(call_id, name, json!({ "input": input }))
                        .with_surface(ToolCallSurface::Freeform),
                );
            }
            _ => {}
        }
    }

    let finish_reason = if length_limited {
        FinishReason::Length
    } else if tool_calls.is_empty() {
        FinishReason::Stop
    } else {
        FinishReason::ToolCalls
    };
    let mut parts = text_parts
        .into_iter()
        .map(|text| ContentPart::Text { text })
        .collect::<Vec<_>>();
    parts.extend(
        tool_calls
            .iter()
            .cloned()
            .map(|call| ContentPart::ToolCall { call }),
    );

    let message = CanonicalMessage::new(MessageRole::Assistant, parts);
    let usage = parse_usage(&response);
    let mut resp = CanonicalModelResponse::new(message, tool_calls, finish_reason);
    if let Some(u) = usage {
        resp = resp.with_usage(u);
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

fn is_length_limited_response(response: &Value) -> bool {
    response.get("status").and_then(Value::as_str) == Some("incomplete")
        && response
            .get("incomplete_details")
            .and_then(|details| details.get("reason"))
            .and_then(Value::as_str)
            == Some("max_output_tokens")
}
