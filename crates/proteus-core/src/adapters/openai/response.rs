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
    if response.get("status").and_then(Value::as_str) == Some("incomplete") {
        let reason = response
            .get("incomplete_details")
            .and_then(|details| details.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(anyhow!("Incomplete response returned, reason: {reason}"));
    }

    let mut parts = Vec::new();
    let mut tool_calls = Vec::new();

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
                            parts.push(ContentPart::Text {
                                text: text.to_owned(),
                            });
                        }
                    }
                }
            }
            Some("reasoning") => {
                let text = item
                    .get("summary")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|summary| {
                        summary.get("type").and_then(Value::as_str) == Some("summary_text")
                    })
                    .filter_map(|summary| summary.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let signature = item
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if !text.is_empty() || signature.is_some() {
                    parts.push(ContentPart::Reasoning { text, signature });
                }
            }
            Some("function_call") => {
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("function_call missing call_id"))?
                    .to_owned();
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("function_call missing name"))?
                    .to_owned();
                let raw_arguments = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                let args = serde_json::from_str(raw_arguments)
                    .unwrap_or_else(|_| Value::String(raw_arguments.to_owned()));
                let call =
                    ToolCall::new(call_id, name, args).with_raw_arguments(raw_arguments.to_owned());
                parts.push(ContentPart::ToolCall { call: call.clone() });
                tool_calls.push(call);
            }
            Some("custom_tool_call") => {
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
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
                let call = ToolCall::new(call_id, name, json!({ "input": input }))
                    .with_surface(ToolCallSurface::Freeform);
                parts.push(ContentPart::ToolCall { call: call.clone() });
                tool_calls.push(call);
            }
            _ => {}
        }
    }

    let finish_reason = if tool_calls.is_empty() {
        FinishReason::Stop
    } else {
        FinishReason::ToolCalls
    };
    let message = CanonicalMessage::new(MessageRole::Assistant, parts);
    let usage = parse_usage(&response);
    let mut resp = CanonicalModelResponse::new(message, tool_calls, finish_reason);
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
