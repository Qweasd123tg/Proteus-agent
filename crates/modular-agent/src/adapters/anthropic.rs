use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures_util::stream;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

use crate::{
    adapters::secrets::read_secret_from_config,
    contracts::{ModelAdapter, ModelEventStream},
    domain::{ModelRef, ToolCall, ToolChoice, ToolSpec},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole, ModelCapabilities, TokenUsage,
    },
};

#[derive(Debug, Clone)]
pub struct AnthropicMessagesClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    api_version: String,
    auth: AnthropicAuth,
}

impl AnthropicMessagesClient {
    pub fn from_provider_config(config: Value) -> Result<Self> {
        let api_key = read_secret_from_config(&config, "ANTHROPIC_API_KEY", "anthropic_api_key")?;
        let base_url = config
            .get("base_url")
            .and_then(Value::as_str)
            .unwrap_or("https://api.anthropic.com")
            .trim_end_matches('/')
            .to_owned();
        let api_version = config
            .get("api_version")
            .and_then(Value::as_str)
            .unwrap_or("2023-06-01")
            .to_owned();
        let auth = AnthropicAuth::from_config(
            config
                .get("auth")
                .and_then(Value::as_str)
                .unwrap_or("x-api-key"),
        )?;

        Ok(Self {
            http: reqwest::Client::new(),
            api_key,
            base_url,
            api_version,
            auth,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum AnthropicAuth {
    XApiKey,
    Bearer,
}

impl AnthropicAuth {
    fn from_config(value: &str) -> Result<Self> {
        match value {
            "x-api-key" | "x_api_key" | "anthropic" => Ok(Self::XApiKey),
            "bearer" | "authorization_bearer" => Ok(Self::Bearer),
            other => Err(anyhow!("unsupported Anthropic auth mode: {other}")),
        }
    }
}

#[async_trait]
impl ModelAdapter for AnthropicMessagesClient {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "anthropic.messages".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities {
            supports_tools: true,
            supports_parallel_tool_calls: true,
            supports_streaming: false,
            supports_json_schema: false,
            supports_system_role: true,
            supports_developer_role: false,
            supports_cache_hints: false,
            supports_reasoning_config: true,
            supports_image_input: false,
            supports_file_input: false,
            max_input_tokens: Some(200_000),
            max_output_tokens: Some(64_000),
        }
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let response = self.complete_response(request).await?;
        Ok(Box::pin(stream::once(async move {
            Ok(crate::model_standard::ModelStreamEvent::Response { response })
        })))
    }
}

impl AnthropicMessagesClient {
    async fn complete_response(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse> {
        let body = to_anthropic_request(&request)?;
        let url = format!("{}/v1/messages", self.base_url);
        let builder = self
            .http
            .post(url)
            .header("anthropic-version", &self.api_version)
            .header(CONTENT_TYPE, "application/json")
            .json(&body);
        let builder = match self.auth {
            AnthropicAuth::XApiKey => builder.header("x-api-key", &self.api_key),
            AnthropicAuth::Bearer => {
                builder.header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            }
        };
        let response = builder.send().await?;

        let status = response.status();
        let response_text = response.text().await?;
        if !status.is_success() {
            return Err(anyhow!("Anthropic API error {status}: {response_text}"));
        }

        let response: Value = serde_json::from_str(&response_text)?;
        from_anthropic_response(response)
    }
}

fn to_anthropic_request(request: &CanonicalModelRequest) -> Result<Value> {
    let mut body = json!({
        "model": request.model.model,
        "max_tokens": request.limits.max_output_tokens.unwrap_or(2048),
        "messages": to_anthropic_messages(&request.messages)?,
    });

    if let Some(system) = joined_instructions(request) {
        body["system"] = Value::String(system);
    }

    if !request.tools.is_empty() {
        body["tools"] = Value::Array(request.tools.iter().map(to_anthropic_tool).collect());
        body["tool_choice"] = match &request.tool_choice {
            ToolChoice::None => json!({ "type": "none" }),
            ToolChoice::Auto => json!({ "type": "auto" }),
            ToolChoice::Required => json!({ "type": "any" }),
            ToolChoice::Tool(name) => json!({ "type": "tool", "name": name }),
            _ => json!({ "type": "auto" }),
        };
    }

    if let Some(temperature) = request.sampling.temperature {
        body["temperature"] = json!(temperature);
    } else if let Some(top_p) = request.sampling.top_p {
        body["top_p"] = json!(top_p);
    }

    Ok(body)
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

fn to_anthropic_tool(tool: &ToolSpec) -> Value {
    json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.input_schema,
    })
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
            ContentPart::ToolCall { call } => blocks.push(json!({
                "type": "tool_use",
                "id": call.id,
                "name": call.name,
                "input": call.args,
            })),
            ContentPart::ToolResult { result } => blocks.push(tool_result_block(result)),
            ContentPart::ReasoningSummary { text } => blocks.push(json!({
                "type": "text",
                "text": format!("Reasoning summary: {text}")
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
    let content = result
        .error
        .clone()
        .unwrap_or_else(|| result.output.clone());
    json!({
        "type": "tool_result",
        "tool_use_id": result.call_id,
        "content": content,
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

fn from_anthropic_response(response: Value) -> Result<CanonicalModelResponse> {
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
            Some("text") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(ContentPart::Text {
                        text: text.to_owned(),
                    });
                }
            }
            Some("tool_use") if accept_tool_calls => {
                let call = ToolCall {
                    id: item
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("tool_use missing id"))?
                        .to_owned(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("tool_use missing name"))?
                        .to_owned(),
                    args: item.get("input").cloned().unwrap_or(Value::Null),
                };
                parts.push(ContentPart::ToolCall { call: call.clone() });
                tool_calls.push(call);
            }
            Some("tool_use") => {}
            _ => {}
        }
    }

    Ok(CanonicalModelResponse {
        message: CanonicalMessage {
            id: crate::domain::new_message_id(),
            role: MessageRole::Assistant,
            parts,
            name: None,
            tool_call_id: None,
            metadata: serde_json::Value::Null,
        },
        tool_calls,
        finish_reason,
        usage: parse_usage(&response),
        provider_metadata: response,
    })
}

fn parse_usage(response: &Value) -> Option<TokenUsage> {
    let usage = response.get("usage")?;
    Some(TokenUsage {
        input_tokens: usage.get("input_tokens")?.as_u64()? as u32,
        output_tokens: usage.get("output_tokens")?.as_u64()? as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_tokens_tool_use_is_not_returned_as_executable_call() {
        let response = json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-test",
            "stop_reason": "max_tokens",
            "content": [
                { "type": "text", "text": "writing file" },
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "write_file",
                    "input": {}
                }
            ],
            "usage": { "input_tokens": 10, "output_tokens": 2048 }
        });

        let canonical = from_anthropic_response(response).unwrap();

        assert_eq!(canonical.finish_reason, FinishReason::Length);
        assert!(canonical.tool_calls.is_empty());
        assert!(
            canonical
                .message
                .parts
                .iter()
                .all(|part| !matches!(part, ContentPart::ToolCall { .. }))
        );
    }

    #[test]
    fn completed_tool_use_is_returned_as_executable_call() {
        let response = json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-test",
            "stop_reason": "tool_use",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "write_file",
                    "input": { "path": "site/index.html", "content": "<html></html>" }
                }
            ],
            "usage": { "input_tokens": 10, "output_tokens": 120 }
        });

        let canonical = from_anthropic_response(response).unwrap();

        assert_eq!(canonical.finish_reason, FinishReason::ToolCalls);
        assert_eq!(canonical.tool_calls.len(), 1);
        assert_eq!(canonical.tool_calls[0].args["path"], "site/index.html");
    }
}
