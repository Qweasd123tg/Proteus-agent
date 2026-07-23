use std::collections::{BTreeSet, HashMap};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use super::{OpenAiPromptCacheConfig, model_profile::OpenAiModelProfile};
use crate::{
    domain::{
        CONTEXT_RENDER_MODE_KEY, CONTEXT_RENDER_MODE_VERBATIM, ContextChunk,
        FileSearchHostedToolConfig, HostedToolConfig, ResponseFormat, ToolCall, ToolCallSurface,
        ToolChoice, ToolSpec, ToolSurface, WebSearchHostedToolConfig,
    },
    model_standard::{CanonicalMessage, CanonicalModelRequest, ContentPart, MessageRole},
};

#[cfg(test)]
pub(super) fn to_openai_request(request: &CanonicalModelRequest) -> Result<Value> {
    let profile = OpenAiModelProfile::from_provider_config(&json!({
        "capabilities": {
            "supports_parallel_tool_calls": true,
            "supports_json_schema": true,
            "supports_reasoning_config": true
        }
    }))?;
    to_openai_request_with_cache(request, &OpenAiPromptCacheConfig::default(), &profile)
}

pub(super) fn to_openai_request_with_cache(
    request: &CanonicalModelRequest,
    prompt_cache: &OpenAiPromptCacheConfig,
    profile: &OpenAiModelProfile,
) -> Result<Value> {
    let mut body = json!({
        "model": request.model.model,
        "input": to_openai_input(&request.messages)?,
        "tool_choice": match &request.tool_choice {
            ToolChoice::None => "none",
            ToolChoice::Auto => "auto",
            ToolChoice::Required => "required",
            ToolChoice::Tool(_) => "auto",
            _ => "auto",
        },
        "parallel_tool_calls": profile.supports_parallel_tool_calls,
        "store": profile.store,
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
        if request
            .tools
            .iter()
            .any(|tool| matches!(tool.surface, ToolSurface::ProviderHosted { .. }))
            && let Some(max_tool_calls) = profile.hosted_tools.max_tool_calls()
        {
            body["max_tool_calls"] = json!(max_tool_calls);
        }
    }

    if let Some(max_output_tokens) = request.limits.max_output_tokens {
        body["max_output_tokens"] = json!(max_output_tokens);
    }

    if let Some(text) = openai_text_controls(&request.response_format, profile) {
        body["text"] = text;
    }

    let mut include = BTreeSet::new();
    if request.reasoning.effort.is_some() || request.reasoning.summary {
        let mut reasoning = serde_json::Map::new();
        if let Some(effort) = &request.reasoning.effort {
            reasoning.insert("effort".to_owned(), Value::String(effort.clone()));
        }
        if request.reasoning.summary {
            reasoning.insert("summary".to_owned(), Value::String("auto".to_owned()));
        }
        body["reasoning"] = Value::Object(reasoning);
        include.insert("reasoning.encrypted_content");
    }
    collect_hosted_tool_includes(&request.tools, &mut include);
    body["include"] = Value::Array(
        include
            .into_iter()
            .map(|value| Value::String(value.to_owned()))
            .collect(),
    );

    apply_openai_prompt_cache(request, prompt_cache, &mut body);
    if let Some(service_tier) = profile.service_tier.as_deref() {
        body["service_tier"] = Value::String(service_tier.to_owned());
    }
    if let Some(client_metadata) = openai_client_metadata(request, profile)? {
        body["client_metadata"] = client_metadata;
    }

    Ok(body)
}

fn openai_text_controls(
    response_format: &ResponseFormat,
    profile: &OpenAiModelProfile,
) -> Option<Value> {
    let mut controls = serde_json::Map::new();
    if let Some(verbosity) = profile.effective_verbosity() {
        controls.insert("verbosity".to_owned(), Value::String(verbosity.to_owned()));
    }
    match response_format {
        ResponseFormat::Text => {}
        ResponseFormat::Json => {
            controls.insert("format".to_owned(), json!({ "type": "json_object" }));
        }
        ResponseFormat::JsonSchema {
            name,
            schema,
            strict,
        } => {
            controls.insert(
                "format".to_owned(),
                json!({
                    "type": "json_schema",
                    "name": name,
                    "schema": schema,
                    "strict": strict,
                }),
            );
        }
        _ => {}
    }
    (!controls.is_empty()).then_some(Value::Object(controls))
}

fn openai_client_metadata(
    request: &CanonicalModelRequest,
    profile: &OpenAiModelProfile,
) -> Result<Option<Value>> {
    let mut metadata = profile.client_metadata.clone();
    metadata.extend(request.client_metadata.clone());
    if metadata.is_empty() {
        Ok(None)
    } else {
        Ok(Some(serde_json::to_value(metadata)?))
    }
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
            .cache
            .routing_key
            .as_deref()
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
        ToolSurface::ProviderHosted { config } => to_openai_hosted_tool(config),
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
        ToolSurface::ProviderHosted { config } => Ok(json!({ "type": config.kind().as_str() })),
        _ => Err(anyhow!(
            "tool '{}' uses unsupported surface for openai.responses",
            tool.name
        )),
    }
}

fn to_openai_hosted_tool(config: &HostedToolConfig) -> Result<Value> {
    match config {
        HostedToolConfig::WebSearch { config } => Ok(to_openai_web_search_tool(config)),
        HostedToolConfig::FileSearch { config } => Ok(to_openai_file_search_tool(config)),
        _ => Err(anyhow!(
            "unsupported provider-hosted tool for openai.responses"
        )),
    }
}

fn to_openai_web_search_tool(config: &WebSearchHostedToolConfig) -> Value {
    let mut value = json!({ "type": "web_search" });
    if let Some(size) = config.search_context_size {
        value["search_context_size"] = Value::String(size.as_str().to_owned());
    }
    if !config.allowed_domains.is_empty() || !config.blocked_domains.is_empty() {
        let mut filters = serde_json::Map::new();
        if !config.allowed_domains.is_empty() {
            filters.insert("allowed_domains".to_owned(), json!(config.allowed_domains));
        }
        if !config.blocked_domains.is_empty() {
            filters.insert("blocked_domains".to_owned(), json!(config.blocked_domains));
        }
        value["filters"] = Value::Object(filters);
    }
    if let Some(external_web_access) = config.external_web_access {
        value["external_web_access"] = Value::Bool(external_web_access);
    }
    value
}

fn to_openai_file_search_tool(config: &FileSearchHostedToolConfig) -> Value {
    let mut value = json!({
        "type": "file_search",
        "vector_store_ids": config.vector_store_ids,
    });
    if let Some(max_num_results) = config.max_num_results {
        value["max_num_results"] = json!(max_num_results);
    }
    value
}

fn collect_hosted_tool_includes<'a>(tools: &'a [ToolSpec], include: &mut BTreeSet<&'a str>) {
    for tool in tools {
        match &tool.surface {
            ToolSurface::ProviderHosted {
                config:
                    HostedToolConfig::WebSearch {
                        config:
                            WebSearchHostedToolConfig {
                                include_sources: true,
                                ..
                            },
                    },
            } => {
                include.insert("web_search_call.action.sources");
            }
            ToolSurface::ProviderHosted {
                config:
                    HostedToolConfig::FileSearch {
                        config:
                            FileSearchHostedToolConfig {
                                include_results: true,
                                ..
                            },
                    },
            } => {
                include.insert("file_search_call.results");
            }
            _ => {}
        }
    }
}

fn to_openai_input(messages: &[CanonicalMessage]) -> Result<Vec<Value>> {
    let mut input = Vec::new();
    let mut tool_call_surfaces = HashMap::new();
    for message in messages {
        for part in &message.parts {
            match &part.payload {
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
                        "text": context_text(chunk)
                    }]
                })),
                ContentPart::ToolCall { call } => {
                    tool_call_surfaces.insert(call.id.clone(), call.surface);
                    match call.surface {
                        ToolCallSurface::Function => input.push(json!({
                            "type": "function_call",
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": match call.raw_arguments.as_deref() {
                                Some(raw) => raw.to_owned(),
                                None => serde_json::to_string(&call.args)?,
                            },
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
                ContentPart::ReasoningSummary { text } => {
                    input.push(openai_reasoning_item(text, None))
                }
                ContentPart::Reasoning { text, signature } => {
                    input.push(openai_reasoning_item(text, signature.as_deref()))
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

fn context_text(chunk: &ContextChunk) -> String {
    if chunk
        .metadata
        .get(CONTEXT_RENDER_MODE_KEY)
        .and_then(Value::as_str)
        == Some(CONTEXT_RENDER_MODE_VERBATIM)
    {
        return chunk.content.clone();
    }
    format!(
        "Context from {}{}:\n{}",
        chunk.source,
        chunk
            .path
            .as_ref()
            .map(|path| format!(" ({})", path.display()))
            .unwrap_or_default(),
        chunk.content
    )
}

fn openai_reasoning_item(summary: &str, encrypted_content: Option<&str>) -> Value {
    let summary = if summary.trim().is_empty() {
        Vec::new()
    } else {
        vec![json!({ "type": "summary_text", "text": summary })]
    };
    json!({
        "type": "reasoning",
        "summary": summary,
        "encrypted_content": encrypted_content,
    })
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
