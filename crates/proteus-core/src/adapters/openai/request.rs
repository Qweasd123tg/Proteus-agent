use std::collections::HashMap;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use super::OpenAiPromptCacheConfig;
use crate::{
    domain::{ToolCall, ToolCallSurface, ToolChoice, ToolSpec, ToolSurface},
    model_standard::{CanonicalMessage, CanonicalModelRequest, ContentPart, MessageRole},
};

#[cfg(test)]
pub(super) fn to_openai_request(request: &CanonicalModelRequest) -> Result<Value> {
    to_openai_request_with_cache(request, &OpenAiPromptCacheConfig::default())
}

pub(super) fn to_openai_request_with_cache(
    request: &CanonicalModelRequest,
    prompt_cache: &OpenAiPromptCacheConfig,
) -> Result<Value> {
    let mut body = json!({
        "model": request.model.model,
        "input": to_openai_input(&request.messages)?,
        "store": false,
    });

    if let Some(instructions) = joined_instructions(request) {
        body["instructions"] = Value::String(instructions);
    }

    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(to_openai_tool)
                .collect::<Result<Vec<_>>>()?,
        );
        body["tool_choice"] = match &request.tool_choice {
            ToolChoice::None => Value::String("none".to_owned()),
            ToolChoice::Auto => Value::String("auto".to_owned()),
            ToolChoice::Required => Value::String("required".to_owned()),
            ToolChoice::Tool(name) => openai_named_tool_choice(request, name)?,
            _ => Value::String("auto".to_owned()),
        };
    }

    if let Some(max_output_tokens) = request.limits.max_output_tokens {
        body["max_output_tokens"] = json!(max_output_tokens);
    }

    if request.response_format == crate::domain::ResponseFormat::Json {
        body["text"] = json!({ "format": { "type": "json_object" } });
    }

    if request.reasoning.effort.is_some() || request.reasoning.summary {
        let mut reasoning = serde_json::Map::new();
        if let Some(effort) = &request.reasoning.effort {
            reasoning.insert("effort".to_owned(), Value::String(effort.clone()));
        }
        if request.reasoning.summary {
            reasoning.insert("summary".to_owned(), Value::String("auto".to_owned()));
        }
        body["reasoning"] = Value::Object(reasoning);
    }

    apply_openai_prompt_cache(request, prompt_cache, &mut body);

    Ok(body)
}

fn apply_openai_prompt_cache(
    request: &CanonicalModelRequest,
    prompt_cache: &OpenAiPromptCacheConfig,
    body: &mut Value,
) {
    if !prompt_cache.enabled || !(request.cache.cache_instructions || request.cache.cache_context) {
        return;
    }

    let key = prompt_cache.key.as_deref().or_else(|| {
        request
            .metadata
            .get("prompt_cache_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    });
    if let Some(key) = key {
        body["prompt_cache_key"] = Value::String(key.to_owned());
    }
    if let Some(retention) = prompt_cache.retention.as_deref() {
        body["prompt_cache_retention"] = Value::String(retention.to_owned());
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

fn to_openai_tool(tool: &ToolSpec) -> Result<Value> {
    match &tool.surface {
        ToolSurface::Function {
            strict,
            output_schema,
        } => {
            let mut value = json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
                "strict": strict,
            });
            if let Some(output_schema) = output_schema {
                value["output_schema"] = output_schema.clone();
            }
            Ok(value)
        }
        ToolSurface::Freeform { format } => Ok(json!({
            "type": "custom",
            "name": tool.name,
            "description": tool.description,
            "format": format,
        })),
        _ => Err(anyhow!(
            "tool '{}' uses unsupported surface for openai.responses",
            tool.name
        )),
    }
}

fn openai_named_tool_choice(request: &CanonicalModelRequest, name: &str) -> Result<Value> {
    let tool = request
        .tools
        .iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| anyhow!("tool_choice references unknown tool '{name}'"))?;
    match &tool.surface {
        ToolSurface::Function { .. } => Ok(json!({ "type": "function", "name": name })),
        ToolSurface::Freeform { .. } => Ok(json!({ "type": "custom", "name": name })),
        _ => Err(anyhow!(
            "tool '{}' uses unsupported surface for openai.responses",
            tool.name
        )),
    }
}

fn to_openai_input(messages: &[CanonicalMessage]) -> Result<Vec<Value>> {
    let mut input = Vec::new();
    let mut tool_call_surfaces = HashMap::new();
    for message in messages {
        for part in &message.parts {
            match part {
                ContentPart::Text { text } => input.push(json!({
                    "type": "message",
                    "role": role_to_openai(&message.role),
                    "content": [{ "type": content_text_type(&message.role), "text": text }]
                })),
                ContentPart::Context { chunk } => input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": format!("Context from {}{}:\n{}",
                            chunk.source,
                            chunk.path.as_ref().map(|path| format!(" ({})", path.display())).unwrap_or_default(),
                            chunk.content
                        )
                    }]
                })),
                ContentPart::ToolCall { call } => {
                    tool_call_surfaces.insert(call.id.clone(), call.surface);
                    match call.surface {
                        ToolCallSurface::Function => input.push(json!({
                            "type": "function_call",
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": serde_json::to_string(&call.args)?,
                        })),
                        ToolCallSurface::Freeform => input.push(json!({
                            "type": "custom_tool_call",
                            "call_id": call.id,
                            "name": call.name,
                            "input": freeform_tool_input(call)?,
                        })),
                        _ => {
                            return Err(anyhow!(
                                "tool call '{}' uses unsupported surface for openai.responses",
                                call.name
                            ));
                        }
                    }
                }
                ContentPart::ToolResult { result } => {
                    let surface = tool_call_surfaces
                        .get(&result.call_id)
                        .copied()
                        .unwrap_or_default();
                    match surface {
                        ToolCallSurface::Function => input.push(json!({
                            "type": "function_call_output",
                            "call_id": result.call_id,
                            "output": result.text_or_status(),
                        })),
                        ToolCallSurface::Freeform => input.push(json!({
                            "type": "custom_tool_call_output",
                            "call_id": result.call_id,
                            "output": result.text_or_status(),
                        })),
                        _ => {
                            return Err(anyhow!(
                                "tool result '{}' uses unsupported surface for openai.responses",
                                result.call_id
                            ));
                        }
                    }
                }
                ContentPart::ReasoningSummary { text } | ContentPart::Reasoning { text, .. } => {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": format!("Reasoning summary: {text}") }]
                    }))
                }
                ContentPart::FileRef { path, content } => input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": format!("File reference: {}\n{}", path.display(), content.clone().unwrap_or_default())
                    }]
                })),
                ContentPart::Patch { patch } => input.push(json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": patch.content }]
                })),
                _ => {}
            }
        }
    }
    Ok(input)
}

fn freeform_tool_input(call: &ToolCall) -> Result<String> {
    call.args
        .get("input")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            anyhow!(
                "freeform tool call '{}' requires string arg 'input'",
                call.name
            )
        })
}

fn role_to_openai(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::Developer => "developer",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        _ => "user",
    }
}

fn content_text_type(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::Assistant => "output_text",
        _ => "input_text",
    }
}
