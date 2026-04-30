use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::{StreamExt, stream};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

use crate::{
    adapters::secrets::read_secret_from_config,
    contracts::{ModelAdapter, ModelEventStream},
    domain::{ModelRef, ToolCall, ToolChoice, ToolSpec},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole, ModelCapabilities, ModelStreamEvent, TokenUsage,
    },
};

#[derive(Debug, Clone)]
pub struct AnthropicMessagesClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    api_version: String,
    auth: AnthropicAuth,
    /// Включает SSE-стрим через `"stream": true` в body. Управляется
    /// полем `stream: true` в provider config, default false.
    stream_enabled: bool,
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
        let stream_enabled = config
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        Ok(Self {
            http: reqwest::Client::new(),
            api_key,
            base_url,
            api_version,
            auth,
            stream_enabled,
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
        ModelCapabilities::empty()
            .with_tools(true)
            .with_parallel_tool_calls(true)
            .with_system_role(true)
            .with_reasoning_config(true)
            .with_streaming(true)
            .with_max_input_tokens(Some(200_000))
            .with_max_output_tokens(Some(64_000))
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        if self.stream_enabled {
            self.stream_response(request).await
        } else {
            let response = self.complete_response(request).await?;
            Ok(Box::pin(stream::once(async move {
                Ok(ModelStreamEvent::Response { response })
            })))
        }
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

    async fn stream_response(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<ModelEventStream> {
        let mut body = to_anthropic_request(&request)?;
        body["stream"] = json!(true);
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
        let response = builder.send().await?.error_for_status()?;

        // Anthropic SSE stateful: content_block_start открывает блок,
        // множество content_block_delta расширяют его, content_block_stop
        // закрывает. Для tool_use input_json_delta приходит инкрементально;
        // собираем всё в state и на message_stop отдаём Response.
        let state = Arc::new(Mutex::new(AnthropicStreamState::default()));
        let events = response
            .bytes_stream()
            .eventsource()
            .flat_map(move |chunk| {
                let state = state.clone();
                let mapped = match chunk {
                    Ok(event) => {
                        let mut guard = state.lock().unwrap();
                        guard.translate(&event.event, &event.data)
                    }
                    Err(error) => vec![ModelStreamEvent::Error {
                        message: format!("sse transport error: {error}"),
                    }],
                };
                stream::iter(mapped.into_iter().map(Ok).collect::<Vec<_>>())
            });
        Ok(Box::pin(events))
    }
}

/// Stateful аккумулятор для Anthropic SSE-потока: копит text parts и
/// tool_use блоки по мере прихода, на `message_stop` отдаёт финальный
/// CanonicalModelResponse.
#[derive(Default)]
struct AnthropicStreamState {
    blocks: Vec<AnthropicBlock>,
    usage: Option<TokenUsage>,
    stop_reason: Option<String>,
    // Anthropic SSE референсит блоки по index, так что нужен index → block mapping.
}

#[derive(Debug, Clone)]
enum AnthropicBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
}

impl AnthropicStreamState {
    fn translate(&mut self, event_type: &str, data: &str) -> Vec<ModelStreamEvent> {
        let Ok(parsed) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        match event_type {
            "content_block_start" => {
                let index = parsed.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let block = parsed.get("content_block");
                let block_type = block
                    .and_then(|b| b.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let new_block = match block_type {
                    "text" => Some(AnthropicBlock::Text {
                        text: String::new(),
                    }),
                    "tool_use" => {
                        let id = block
                            .and_then(|b| b.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let name = block
                            .and_then(|b| b.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        Some(AnthropicBlock::ToolUse {
                            id,
                            name,
                            input_json: String::new(),
                        })
                    }
                    _ => None,
                };
                if let Some(block) = new_block {
                    if self.blocks.len() <= index {
                        self.blocks.resize(index + 1, AnthropicBlock::Text {
                            text: String::new(),
                        });
                    }
                    self.blocks[index] = block;
                }
                Vec::new()
            }
            "content_block_delta" => {
                let index = parsed.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = parsed.get("delta");
                let delta_type = delta
                    .and_then(|d| d.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match delta_type {
                    "text_delta" => {
                        let text = delta
                            .and_then(|d| d.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if let Some(AnthropicBlock::Text { text: buf }) =
                            self.blocks.get_mut(index)
                        {
                            buf.push_str(text);
                        }
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![ModelStreamEvent::TextDelta {
                                text: text.to_owned(),
                            }]
                        }
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let call_id = if let Some(AnthropicBlock::ToolUse {
                            id, input_json, ..
                        }) = self.blocks.get_mut(index)
                        {
                            input_json.push_str(partial);
                            id.clone()
                        } else {
                            return Vec::new();
                        };
                        if partial.is_empty() {
                            Vec::new()
                        } else {
                            vec![ModelStreamEvent::ToolCallDelta {
                                call_id,
                                name: None,
                                args_delta: partial.to_owned(),
                            }]
                        }
                    }
                    _ => Vec::new(),
                }
            }
            "content_block_stop" => Vec::new(),
            "message_delta" => {
                if let Some(stop) = parsed
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    self.stop_reason = Some(stop.to_owned());
                }
                if let Some(usage) = parsed.get("usage") {
                    let input = usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    let output = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    self.usage = Some(TokenUsage::new(input as u32, output as u32));
                }
                Vec::new()
            }
            "message_stop" => {
                vec![self.finalise()]
            }
            "error" => {
                let message = parsed
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| "unknown anthropic error".to_owned());
                vec![ModelStreamEvent::Error { message }]
            }
            _ => Vec::new(),
        }
    }

    fn finalise(&mut self) -> ModelStreamEvent {
        let mut parts = Vec::new();
        let mut tool_calls = Vec::new();
        for block in self.blocks.drain(..) {
            match block {
                AnthropicBlock::Text { text } if !text.is_empty() => {
                    parts.push(ContentPart::Text { text });
                }
                AnthropicBlock::Text { .. } => {}
                AnthropicBlock::ToolUse {
                    id,
                    name,
                    input_json,
                } => {
                    let args = if input_json.is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_str(&input_json).unwrap_or(Value::Null)
                    };
                    let call = ToolCall::new(id, name, args);
                    parts.push(ContentPart::ToolCall { call: call.clone() });
                    tool_calls.push(call);
                }
            }
        }

        let finish_reason = match self.stop_reason.as_deref() {
            Some("end_turn") | Some("stop_sequence") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::Length,
            Some("tool_use") if !tool_calls.is_empty() => FinishReason::ToolCalls,
            _ if !tool_calls.is_empty() => FinishReason::ToolCalls,
            _ => FinishReason::Stop,
        };
        let message = CanonicalMessage::new(MessageRole::Assistant, parts);
        let mut resp = CanonicalModelResponse::new(message, tool_calls, finish_reason);
        if let Some(u) = self.usage.take() {
            resp = resp.with_usage(u);
        }
        ModelStreamEvent::Response { response: resp }
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
    Some(TokenUsage::new(
        usage.get("input_tokens")?.as_u64()? as u32,
        usage.get("output_tokens")?.as_u64()? as u32,
    ))
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

    // Хелпер: проигрывает SSE-трассу через стейт-парсер и возвращает
    // список всех эмитнутых ModelStreamEvent'ов.
    fn run_trace(trace: &[(&str, Value)]) -> Vec<ModelStreamEvent> {
        let mut state = AnthropicStreamState::default();
        let mut out = Vec::new();
        for (event_type, data) in trace {
            out.extend(state.translate(event_type, &data.to_string()));
        }
        out
    }

    #[test]
    fn stream_trace_plain_text_emits_deltas_and_final_response() {
        let trace = vec![
            (
                "content_block_start",
                json!({
                    "index": 0,
                    "content_block": { "type": "text", "text": "" }
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "Hello" }
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "index": 0,
                    "delta": { "type": "text_delta", "text": " world" }
                }),
            ),
            ("content_block_stop", json!({ "index": 0 })),
            (
                "message_delta",
                json!({
                    "delta": { "stop_reason": "end_turn" },
                    "usage": { "input_tokens": 10, "output_tokens": 2 }
                }),
            ),
            ("message_stop", json!({})),
        ];
        let events = run_trace(&trace);

        let kinds: Vec<&str> = events
            .iter()
            .map(|e| match e {
                ModelStreamEvent::TextDelta { .. } => "text",
                ModelStreamEvent::Response { .. } => "response",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["text", "text", "response"]);

        if let ModelStreamEvent::Response { response } = events.last().unwrap() {
            assert_eq!(response.finish_reason, FinishReason::Stop);
            let text = response
                .message
                .parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            assert_eq!(text, "Hello world");
            assert_eq!(response.usage.as_ref().unwrap().output_tokens, 2);
        } else {
            panic!("last event must be Response");
        }
    }

    #[test]
    fn stream_trace_tool_use_accumulates_input_json() {
        let trace = vec![
            (
                "content_block_start",
                json!({
                    "index": 0,
                    "content_block": {
                        "type": "tool_use",
                        "id": "toolu_abc",
                        "name": "write_file",
                        "input": {}
                    }
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "index": 0,
                    "delta": { "type": "input_json_delta", "partial_json": "{\"path\":" }
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "index": 0,
                    "delta": { "type": "input_json_delta", "partial_json": "\"x.txt\"}" }
                }),
            ),
            ("content_block_stop", json!({ "index": 0 })),
            (
                "message_delta",
                json!({
                    "delta": { "stop_reason": "tool_use" }
                }),
            ),
            ("message_stop", json!({})),
        ];
        let events = run_trace(&trace);

        let tool_deltas: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ModelStreamEvent::ToolCallDelta {
                    call_id,
                    args_delta,
                    ..
                } => Some((call_id.clone(), args_delta.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(tool_deltas.len(), 2);
        assert_eq!(tool_deltas[0].0, "toolu_abc");
        assert_eq!(
            tool_deltas.iter().map(|(_, d)| d.as_str()).collect::<Vec<_>>(),
            vec!["{\"path\":", "\"x.txt\"}"]
        );

        match events.last().unwrap() {
            ModelStreamEvent::Response { response } => {
                assert_eq!(response.finish_reason, FinishReason::ToolCalls);
                assert_eq!(response.tool_calls.len(), 1);
                assert_eq!(response.tool_calls[0].id, "toolu_abc");
                assert_eq!(response.tool_calls[0].args["path"], "x.txt");
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn stream_trace_error_event() {
        let events = run_trace(&[(
            "error",
            json!({ "error": { "type": "overloaded_error", "message": "overloaded" } }),
        )]);
        match events.as_slice() {
            [ModelStreamEvent::Error { message }] => assert_eq!(message, "overloaded"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn stream_trace_unknown_events_are_ignored() {
        let events = run_trace(&[("ping", json!({}))]);
        assert!(events.is_empty());
    }
}
