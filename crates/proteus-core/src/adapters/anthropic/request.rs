use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use super::AnthropicPromptCacheConfig;
use crate::{
    domain::{ToolCallSurface, ToolChoice, ToolSpec, ToolSurface},
    model_standard::{CanonicalMessage, CanonicalModelRequest, ContentPart, MessageRole},
};

#[cfg(test)]
pub(super) fn to_anthropic_request(request: &CanonicalModelRequest) -> Result<Value> {
    to_anthropic_request_with_cache(request, &AnthropicPromptCacheConfig::default())
}

pub(super) fn to_anthropic_request_with_cache(
    request: &CanonicalModelRequest,
    prompt_cache: &AnthropicPromptCacheConfig,
) -> Result<Value> {
    let mut body = json!({
        "model": request.model.model,
        "max_tokens": request.limits.max_output_tokens.unwrap_or(2048),
        "messages": to_anthropic_messages(&request.messages)?,
    });

    let cache_control = anthropic_cache_control(request, prompt_cache);
    let system = joined_instructions(request);
    let system_cache_control = cache_control
        .as_ref()
        .filter(|_| request.cache.cache_instructions && system.is_some());
    let mut has_stable_cache_breakpoint = system_cache_control.is_some();
    if let Some(system) = system {
        body["system"] = anthropic_system_value(system, system_cache_control);
    }

    if !request.tools.is_empty() {
        let tool_cache_index = if request.cache.cache_instructions && system_cache_control.is_none()
        {
            Some(request.tools.len().saturating_sub(1))
        } else {
            None
        };
        has_stable_cache_breakpoint |= tool_cache_index.is_some();
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .enumerate()
                .map(|(index, tool)| {
                    let tool_cache_control = cache_control
                        .as_ref()
                        .filter(|_| tool_cache_index == Some(index));
                    to_anthropic_tool(tool, tool_cache_control)
                })
                .collect::<Result<Vec<_>>>()?,
        );
        body["tool_choice"] = match &request.tool_choice {
            ToolChoice::None => json!({ "type": "none" }),
            ToolChoice::Auto => json!({ "type": "auto" }),
            ToolChoice::Required => json!({ "type": "any" }),
            ToolChoice::Tool(name) => json!({ "type": "tool", "name": name }),
            _ => json!({ "type": "auto" }),
        };
    }
    if request.cache.cache_context
        && !has_stable_cache_breakpoint
        && let Some(cache_control) = cache_control.as_ref()
    {
        body["cache_control"] = cache_control.clone();
    }

    let thinking_requested = request.reasoning.budget_tokens.is_some() || request.reasoning.summary;
    if !thinking_requested {
        if let Some(temperature) = request.sampling.temperature {
            body["temperature"] = json!(temperature);
        } else if let Some(top_p) = request.sampling.top_p {
            body["top_p"] = json!(top_p);
        }
    }

    if let Some(effort) = &request.reasoning.effort {
        body["output_config"] = json!({ "effort": effort });
    }

    if thinking_requested {
        let mut thinking = serde_json::Map::new();
        if let Some(budget_tokens) = request.reasoning.budget_tokens {
            thinking.insert("type".to_owned(), Value::String("enabled".to_owned()));
            thinking.insert("budget_tokens".to_owned(), json!(budget_tokens));
        } else {
            thinking.insert("type".to_owned(), Value::String("adaptive".to_owned()));
        }
        if request.reasoning.summary {
            thinking.insert("display".to_owned(), Value::String("summarized".to_owned()));
        }
        body["thinking"] = Value::Object(thinking);
    }

    Ok(body)
}

fn anthropic_cache_control(
    request: &CanonicalModelRequest,
    prompt_cache: &AnthropicPromptCacheConfig,
) -> Option<Value> {
    if !prompt_cache.enabled || !(request.cache.cache_instructions || request.cache.cache_context) {
        return None;
    }
    let mut cache = serde_json::Map::new();
    cache.insert("type".to_owned(), Value::String("ephemeral".to_owned()));
    if let Some(ttl) = prompt_cache.ttl.as_deref().filter(|ttl| *ttl != "5m") {
        cache.insert("ttl".to_owned(), Value::String(ttl.to_owned()));
    }
    Some(Value::Object(cache))
}

fn anthropic_system_value(system: String, cache_control: Option<&Value>) -> Value {
    if let Some(cache_control) = cache_control {
        json!([{
            "type": "text",
            "text": system,
            "cache_control": cache_control,
        }])
    } else {
        Value::String(system)
    }
}

fn joined_instructions(request: &CanonicalModelRequest) -> Option<String> {
    let mut instructions = request.instructions.clone();
    instructions.sort_by_key(|instruction| std::cmp::Reverse(instruction.priority));
    let text = instructions
        .into_iter()
        .map(|instruction| instruction.text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if text.is_empty() { None } else { Some(text) }
}

fn to_anthropic_tool(tool: &ToolSpec, cache_control: Option<&Value>) -> Result<Value> {
    match &tool.surface {
        ToolSurface::Function { .. } => {
            let mut value = json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            });
            if let Some(cache_control) = cache_control {
                value["cache_control"] = cache_control.clone();
            }
            Ok(value)
        }
        ToolSurface::Freeform { .. } => Err(anyhow!(
            "tool '{}' uses freeform surface, which anthropic.messages does not support",
            tool.name
        )),
        _ => Err(anyhow!(
            "tool '{}' uses unsupported surface for anthropic.messages",
            tool.name
        )),
    }
}

fn to_anthropic_messages(messages: &[CanonicalMessage]) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    let mut pending_tool_results = Vec::new();

    for message in messages {
        if message.role == MessageRole::Tool {
            pending_tool_results.extend(tool_result_blocks(message));
            continue;
        }

        if !pending_tool_results.is_empty() {
            push_message(&mut out, "user", std::mem::take(&mut pending_tool_results));
        }

        let role = anthropic_role(message);
        let blocks = anthropic_content_blocks(message)?;
        if !blocks.is_empty() {
            push_message(&mut out, role, blocks);
        }
    }

    if !pending_tool_results.is_empty() {
        push_message(&mut out, "user", pending_tool_results);
    }

    Ok(out)
}

fn anthropic_role(message: &CanonicalMessage) -> &'static str {
    if message
        .parts
        .iter()
        .any(|part| matches!(part, ContentPart::ToolResult { .. }))
    {
        return "user";
    }

    match message.role {
        MessageRole::Assistant => "assistant",
        _ => "user",
    }
}

fn anthropic_content_blocks(message: &CanonicalMessage) -> Result<Vec<Value>> {
    let mut blocks = Vec::new();
    for part in &message.parts {
        match part {
            ContentPart::Text { text } => blocks.push(json!({ "type": "text", "text": text })),
            ContentPart::Context { chunk } => blocks.push(json!({
                "type": "text",
                "text": format!(
                    "Context from {}{}:\n{}",
                    chunk.source,
                    chunk
                        .path
                        .as_ref()
                        .map(|path| format!(" ({})", path.display()))
                        .unwrap_or_default(),
                    chunk.content
                )
            })),
            ContentPart::ToolCall { call } => match call.surface {
                ToolCallSurface::Function => blocks.push(json!({
                    "type": "tool_use",
                    "id": call.id,
                    "name": call.name,
                    "input": call.args,
                })),
                ToolCallSurface::Freeform => {
                    return Err(anyhow!(
                        "tool call '{}' uses freeform surface, which anthropic.messages does not support",
                        call.name
                    ));
                }
                _ => {
                    return Err(anyhow!(
                        "tool call '{}' uses unsupported surface for anthropic.messages",
                        call.name
                    ));
                }
            },
            ContentPart::ToolResult { result } => blocks.push(tool_result_block(result)),
            ContentPart::ReasoningSummary { text } => blocks.push(json!({
                "type": "text",
                "text": format!("Reasoning summary: {text}")
            })),
            ContentPart::Reasoning { text, signature } => blocks.push(json!({
                "type": "thinking",
                "thinking": text,
                "signature": signature.clone().unwrap_or_default(),
            })),
            ContentPart::FileRef { path, content } => blocks.push(json!({
                "type": "text",
                "text": format!(
                    "File reference: {}\n{}",
                    path.display(),
                    content.clone().unwrap_or_default()
                )
            })),
            ContentPart::Patch { patch } => blocks.push(json!({
                "type": "text",
                "text": patch.content
            })),
            _ => {}
        }
    }
    Ok(blocks)
}

fn tool_result_blocks(message: &CanonicalMessage) -> Vec<Value> {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::ToolResult { result } => Some(tool_result_block(result)),
            _ => None,
        })
        .collect()
}

fn tool_result_block(result: &crate::domain::ToolResult) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": result.call_id,
        "content": result.text_or_status(),
        "is_error": !result.ok,
    })
}

fn push_message(out: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if let Some(last) = out.last_mut() {
        let same_role = last.get("role").and_then(Value::as_str) == Some(role);
        if same_role && let Some(content) = last.get_mut("content").and_then(Value::as_array_mut) {
            content.extend(blocks);
            return;
        }
    }

    out.push(json!({ "role": role, "content": blocks }));
}
