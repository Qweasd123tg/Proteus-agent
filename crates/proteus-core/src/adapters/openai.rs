use anyhow::{Result, anyhow};
use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::{StreamExt, stream};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

use crate::{
    adapters::{http_retry::send_with_transport_retry, secrets::read_secret_from_config},
    contracts::{ModelAdapter, ModelEventStream},
    domain::{ModelRef, ToolCall, ToolChoice, ToolSpec},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, FinishReason,
        MessageRole, ModelCapabilities, ModelStreamEvent, TokenUsage,
    },
};

/// Контекстное окно по умолчанию для семейства gpt-5, когда провайдер не
/// задал `max_input_tokens` явно. Адаптер общий на все openai-модели, поэтому
/// это лишь fallback — точное значение задаётся в provider config.
const DEFAULT_OPENAI_MAX_INPUT_TOKENS: u32 = 272_000;

#[derive(Debug, Clone)]
pub struct OpenAiResponsesClient {
    http: reqwest::Client,
    secret_config: Value,
    base_url: String,
    /// Включает SSE-стрим на `/responses`. Управляется через поле
    /// `stream` в provider config. Provider profiles по умолчанию включают
    /// streaming; `stream = false` оставляет non-stream fallback.
    stream_enabled: bool,
    /// Потолок контекстного окна (`max_input_tokens` в provider config).
    /// Сообщается в capabilities и питает индикатор заполнения контекста.
    max_input_tokens: u32,
}

impl OpenAiResponsesClient {
    pub fn from_provider_config(config: Value) -> Result<Self> {
        let base_url = config
            .get("base_url")
            .and_then(Value::as_str)
            .unwrap_or("https://api.openai.com/v1")
            .trim_end_matches('/')
            .to_owned();
        let stream_enabled = config
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let max_input_tokens = config
            .get("max_input_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_OPENAI_MAX_INPUT_TOKENS);

        Ok(Self {
            http: reqwest::Client::new(),
            secret_config: config,
            base_url,
            stream_enabled,
            max_input_tokens,
        })
    }
}

#[async_trait]
impl ModelAdapter for OpenAiResponsesClient {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "openai.responses".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
            .with_tools(true)
            .with_parallel_tool_calls(true)
            .with_json_schema(true)
            .with_system_role(true)
            .with_developer_role(true)
            .with_reasoning_config(true)
            .with_streaming(true)
            .with_max_input_tokens(Some(self.max_input_tokens))
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

impl OpenAiResponsesClient {
    async fn complete_response(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse> {
        let body = to_openai_request(&request)?;
        let url = format!("{}/responses", self.base_url);
        let api_key = self.api_key()?;
        let response: Value =
            send_with_transport_retry(|| self.request_builder(&url, &body, &api_key))
                .await?
                .error_for_status()?
                .json()
                .await?;

        from_openai_response(response)
    }

    async fn stream_response(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let mut body = to_openai_request(&request)?;
        body["stream"] = json!(true);
        let url = format!("{}/responses", self.base_url);
        let api_key = self.api_key()?;
        let response = send_with_transport_retry(|| self.request_builder(&url, &body, &api_key))
            .await?
            .error_for_status()?;

        // reqwest bytes_stream → eventsource-stream Event → наши ModelStreamEvent.
        // State-parser хранит накопленные text parts / tool_calls / usage и
        // выплёвывает финальный `Response` на event `response.completed`.
        let client = self.clone();
        let fallback_request = request.clone();
        let mut sse = response.bytes_stream().eventsource();
        let events = async_stream::stream! {
            while let Some(chunk) = sse.next().await {
                match chunk {
                    Ok(event) => {
                        let mut saw_response = false;
                        for mapped in translate_sse_event(&event.event, &event.data) {
                            if matches!(mapped, ModelStreamEvent::Response { .. }) {
                                saw_response = true;
                            }
                            yield Ok(mapped);
                        }
                        if saw_response {
                            break;
                        }
                    }
                    Err(error) => {
                        match client.complete_response(fallback_request).await {
                            Ok(response) => yield Ok(ModelStreamEvent::Response { response }),
                            Err(fallback_error) => yield Ok(ModelStreamEvent::Error {
                                message: format!(
                                    "sse transport error: {error}; non-stream fallback failed: {fallback_error}"
                                ),
                            }),
                        }
                        break;
                    }
                }
            }
        };
        Ok(Box::pin(events))
    }

    fn api_key(&self) -> Result<String> {
        read_secret_from_config(&self.secret_config, "OPENAI_API_KEY", "openai_api_key")
    }

    fn request_builder(&self, url: &str, body: &Value, api_key: &str) -> reqwest::RequestBuilder {
        self.http
            .post(url)
            .header(AUTHORIZATION, format!("Bearer {api_key}"))
            .header(CONTENT_TYPE, "application/json")
            .json(body)
    }
}

/// Трансляция одного SSE event'а от OpenAI Responses API в наши
/// `ModelStreamEvent`. Вариантов много; всё что не распознали —
/// игнорируем (возвращаем пустой вектор), это безопасно потому что
/// финальный `Response` приходит на `response.completed`.
fn translate_sse_event(event_type: &str, data: &str) -> Vec<ModelStreamEvent> {
    // [DONE] sentinel у OpenAI не используется в Responses API, но на
    // всякий случай — безопасный фаст-path.
    if data == "[DONE]" {
        return Vec::new();
    }
    let Ok(parsed) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    match event_type {
        "response.output_text.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(Value::as_str) {
                return vec![ModelStreamEvent::TextDelta {
                    text: delta.to_owned(),
                }];
            }
            Vec::new()
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_summary.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(Value::as_str) {
                return vec![ModelStreamEvent::ReasoningSummaryDelta {
                    text: delta.to_owned(),
                }];
            }
            Vec::new()
        }
        "response.function_call_arguments.delta" => {
            let call_id = parsed
                .get("item_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let args_delta = parsed
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if call_id.is_empty() || args_delta.is_empty() {
                return Vec::new();
            }
            vec![ModelStreamEvent::ToolCallDelta {
                call_id,
                name: None,
                args_delta,
            }]
        }
        "response.completed" => {
            // В payload-е объект полного `response`, парсим через
            // существующий `from_openai_response`. Если парсинг упал —
            // эмитим Error, чтобы drain-loop не ждал вечно.
            let response_value = parsed.get("response").cloned().unwrap_or(parsed);
            match from_openai_response(response_value) {
                Ok(response) => vec![ModelStreamEvent::Response { response }],
                Err(error) => vec![ModelStreamEvent::Error {
                    message: format!("failed to parse final response: {error}"),
                }],
            }
        }
        "response.error" | "error" => {
            let message = parsed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    parsed
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "unknown openai error".to_owned());
            vec![ModelStreamEvent::Error { message }]
        }
        _ => Vec::new(),
    }
}

fn to_openai_request(request: &CanonicalModelRequest) -> Result<Value> {
    let mut body = json!({
        "model": request.model.model,
        "input": to_openai_input(&request.messages)?,
        "store": false,
    });

    if let Some(instructions) = joined_instructions(request) {
        body["instructions"] = Value::String(instructions);
    }

    if !request.tools.is_empty() {
        body["tools"] = Value::Array(request.tools.iter().map(to_openai_tool).collect());
        body["tool_choice"] = match &request.tool_choice {
            ToolChoice::None => Value::String("none".to_owned()),
            ToolChoice::Auto => Value::String("auto".to_owned()),
            ToolChoice::Required => Value::String("required".to_owned()),
            ToolChoice::Tool(name) => json!({ "type": "function", "name": name }),
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

fn to_openai_tool(tool: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.input_schema,
        "strict": false,
    })
}

fn to_openai_input(messages: &[CanonicalMessage]) -> Result<Vec<Value>> {
    let mut input = Vec::new();
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
                ContentPart::ToolCall { call } => input.push(json!({
                    "type": "function_call",
                    "call_id": call.id,
                    "name": call.name,
                    "arguments": serde_json::to_string(&call.args)?,
                })),
                ContentPart::ToolResult { result } => input.push(json!({
                    "type": "function_call_output",
                    "call_id": result.call_id,
                    "output": result.text_or_status(),
                })),
                ContentPart::ReasoningSummary { text } | ContentPart::Reasoning { text, .. } => input.push(json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": format!("Reasoning summary: {text}") }]
                })),
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

fn from_openai_response(response: Value) -> Result<CanonicalModelResponse> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ModelLimits, ReasoningConfig, ResponseFormat, SamplingConfig, ToolSafety};

    #[test]
    fn provider_config_does_not_require_secret_until_request() {
        let client = OpenAiResponsesClient::from_provider_config(json!({
            "api_key_env": "__PROTEUS_TEST_MISSING_OPENAI_KEY",
            "stream": false
        }))
        .expect("adapter should build without reading env secret");

        assert!(!client.stream_enabled);
    }

    #[test]
    fn completed_function_call_is_returned_as_executable_call() {
        let response = json!({
            "id": "resp_1",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "write_file",
                    "arguments": "{\"path\":\"site/index.html\",\"content\":\"<html></html>\"}"
                }
            ],
            "usage": { "input_tokens": 10, "output_tokens": 120 }
        });

        let canonical = from_openai_response(response).unwrap();

        assert_eq!(canonical.finish_reason, FinishReason::ToolCalls);
        assert_eq!(canonical.tool_calls.len(), 1);
        assert_eq!(canonical.tool_calls[0].id, "call_1");
        assert_eq!(canonical.tool_calls[0].name, "write_file");
        assert_eq!(canonical.tool_calls[0].args["path"], "site/index.html");
        assert!(
            canonical
                .message
                .parts
                .iter()
                .any(|part| matches!(part, ContentPart::ToolCall { .. }))
        );
    }

    #[test]
    fn response_usage_includes_cache_and_reasoning_details() {
        let response = json!({
            "id": "resp_1",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "hello" }]
                }
            ],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 20,
                "input_tokens_details": { "cached_tokens": 30 },
                "output_tokens_details": { "reasoning_tokens": 7 }
            }
        });

        let canonical = from_openai_response(response).unwrap();
        let usage = canonical.usage.expect("usage");

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.cached_input_tokens, Some(30));
        assert_eq!(usage.reasoning_output_tokens, Some(7));
    }

    #[test]
    fn incomplete_max_output_tokens_function_call_is_not_returned_as_executable_call() {
        let response = json!({
            "id": "resp_1",
            "object": "response",
            "status": "incomplete",
            "incomplete_details": { "reason": "max_output_tokens" },
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "writing file" }]
                },
                {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "write_file",
                    "arguments": "{\"path\":\"site/index.html\"}"
                }
            ],
            "usage": { "input_tokens": 10, "output_tokens": 2048 }
        });

        let canonical = from_openai_response(response).unwrap();

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
    fn request_serializes_tools_tool_choice_reasoning_and_json_format() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("openai", "gpt-test"),
            vec![CanonicalMessage::text(MessageRole::User, "write a file")],
        )
        .with_tools(vec![
            ToolSpec::new(
                "write_file",
                "Write a file",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }),
                ToolSafety::WritesFiles,
            )
            .with_timeout(1_000),
        ])
        .with_tool_choice(ToolChoice::Tool("write_file".to_owned()))
        .with_response_format(ResponseFormat::Json)
        .with_sampling(SamplingConfig::new(Some(0.2), Some(0.9)))
        .with_reasoning(ReasoningConfig::new(Some("medium".to_owned()), true))
        .with_limits(ModelLimits::new(None, Some(123)));

        let body = to_openai_request(&request).unwrap();

        assert_eq!(body["model"], "gpt-test");
        assert_eq!(body["store"], false);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "write_file");
        assert_eq!(body["tools"][0]["parameters"]["required"][0], "path");
        assert_eq!(body["tools"][0]["strict"], false);
        assert_eq!(
            body["tool_choice"],
            json!({ "type": "function", "name": "write_file" })
        );
        assert_eq!(body["text"]["format"]["type"], "json_object");
        assert_eq!(
            body["reasoning"],
            json!({ "effort": "medium", "summary": "auto" })
        );
        assert_eq!(body["max_output_tokens"], 123);
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
    }

    #[test]
    fn translate_sse_text_delta() {
        let events = translate_sse_event(
            "response.output_text.delta",
            &json!({ "delta": "hello" }).to_string(),
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            ModelStreamEvent::TextDelta { text } => assert_eq!(text, "hello"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn translate_sse_reasoning_delta_both_variants() {
        for name in [
            "response.reasoning_summary_text.delta",
            "response.reasoning_summary.delta",
        ] {
            let events = translate_sse_event(name, &json!({ "delta": "thinking" }).to_string());
            assert_eq!(events.len(), 1, "{name}");
            assert!(matches!(
                &events[0],
                ModelStreamEvent::ReasoningSummaryDelta { .. }
            ));
        }
    }

    #[test]
    fn translate_sse_function_call_delta() {
        let events = translate_sse_event(
            "response.function_call_arguments.delta",
            &json!({ "item_id": "call_1", "delta": "{\"a\"" }).to_string(),
        );
        match events.as_slice() {
            [
                ModelStreamEvent::ToolCallDelta {
                    call_id,
                    name,
                    args_delta,
                },
            ] => {
                assert_eq!(call_id, "call_1");
                assert_eq!(name, &None);
                assert_eq!(args_delta, "{\"a\"");
            }
            other => panic!("expected single ToolCallDelta, got {other:?}"),
        }
    }

    #[test]
    fn translate_sse_completed_emits_final_response() {
        let data = json!({
            "response": {
                "id": "resp_1",
                "object": "response",
                "status": "completed",
                "output": [
                    { "type": "message", "content": [{ "type": "output_text", "text": "done" }] }
                ],
                "usage": { "input_tokens": 5, "output_tokens": 1 }
            }
        });
        let events = translate_sse_event("response.completed", &data.to_string());
        match events.as_slice() {
            [ModelStreamEvent::Response { response }] => {
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
                assert_eq!(text, "done");
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn translate_sse_error_event() {
        let events = translate_sse_event(
            "response.error",
            &json!({ "error": { "message": "boom" } }).to_string(),
        );
        match events.as_slice() {
            [ModelStreamEvent::Error { message }] => assert_eq!(message, "boom"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn translate_sse_unknown_event_is_ignored() {
        let events = translate_sse_event("response.weird.thing", "{}");
        assert!(events.is_empty());
    }

    #[test]
    fn translate_sse_done_sentinel_ignored() {
        let events = translate_sse_event("message", "[DONE]");
        assert!(events.is_empty());
    }
}
