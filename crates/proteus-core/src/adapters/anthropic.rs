use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::{StreamExt, stream as futures_stream};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

use crate::{
    adapters::{
        http_retry::send_with_transport_retry,
        secrets::{read_config_string_or_default, read_secret_from_config},
    },
    contracts::{ModelAdapter, ModelEventStream},
    domain::ModelRef,
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, ModelCapabilities, ModelStreamEvent,
    },
};

#[cfg(test)]
use crate::{
    domain::{ToolCall, ToolCallSurface, ToolSpec, ToolSurface},
    model_standard::{CanonicalMessage, ContentPart, FinishReason, MessageRole},
};

mod request;
mod response;
mod sanitize;
mod stream;

#[cfg(test)]
use request::to_anthropic_request;
use request::to_anthropic_request_with_cache;
use response::from_anthropic_response;
#[cfg(test)]
use sanitize::sanitize_provider_text;
use stream::AnthropicStreamState;

#[derive(Debug, Clone)]
pub struct AnthropicMessagesClient {
    http: reqwest::Client,
    secret_config: Value,
    base_url: String,
    api_version: String,
    auth: AnthropicAuth,
    /// Включает SSE-стрим через `"stream": true` в body. Управляется
    /// полем `stream` в provider config; provider profiles включают его
    /// по умолчанию, `stream = false` оставляет non-stream fallback.
    stream_enabled: bool,
    prompt_cache: AnthropicPromptCacheConfig,
}

impl AnthropicMessagesClient {
    pub fn from_provider_config(config: Value) -> Result<Self> {
        let base_url = read_config_string_or_default(
            &config,
            "base_url",
            "https://api.anthropic.com",
            "base_url",
        )?
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
        let prompt_cache = AnthropicPromptCacheConfig::from_provider_config(&config);

        Ok(Self {
            http: reqwest::Client::new(),
            secret_config: config,
            base_url,
            api_version,
            auth,
            stream_enabled,
            prompt_cache,
        })
    }
}

pub fn build_anthropic_messages_adapter(config: Value) -> Result<Arc<dyn ModelAdapter>> {
    Ok(Arc::new(AnthropicMessagesClient::from_provider_config(
        config,
    )?))
}

#[derive(Debug, Clone, Default)]
struct AnthropicPromptCacheConfig {
    enabled: bool,
    ttl: Option<String>,
}

impl AnthropicPromptCacheConfig {
    fn from_provider_config(config: &Value) -> Self {
        Self {
            enabled: config
                .get("prompt_cache")
                .or_else(|| config.get("prompt_caching"))
                .and_then(Value::as_bool)
                .unwrap_or(true),
            ttl: non_empty_config_string(config, "prompt_cache_ttl"),
        }
    }
}

fn non_empty_config_string(config: &Value, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
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
            .with_cache_hints(true)
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
            Ok(Box::pin(futures_stream::once(async move {
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
        let body = to_anthropic_request_with_cache(&request, &self.prompt_cache)?;
        let url = format!("{}/v1/messages", self.base_url);
        let api_key = self.api_key()?;
        let response =
            send_with_transport_retry(|| self.request_builder(&url, &body, &api_key)).await?;

        let status = response.status();
        let response_text = response.text().await?;
        if !status.is_success() {
            return Err(anyhow!("Anthropic API error {status}: {response_text}"));
        }

        let response: Value = serde_json::from_str(&response_text)?;
        from_anthropic_response(response)
    }

    async fn stream_response(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let mut body = to_anthropic_request_with_cache(&request, &self.prompt_cache)?;
        body["stream"] = json!(true);
        let url = format!("{}/v1/messages", self.base_url);
        let api_key = self.api_key()?;
        let response = send_with_transport_retry(|| self.request_builder(&url, &body, &api_key))
            .await?
            .error_for_status()?;

        // Anthropic SSE stateful: content_block_start открывает блок,
        // множество content_block_delta расширяют его, content_block_stop
        // закрывает. Для tool_use input_json_delta приходит инкрементально;
        // собираем всё в state и на message_stop отдаём Response.
        let client = self.clone();
        let fallback_request = request.clone();
        let state = Arc::new(Mutex::new(AnthropicStreamState::default()));
        let mut sse = response.bytes_stream().eventsource();
        let events = async_stream::stream! {
            while let Some(chunk) = sse.next().await {
                match chunk {
                    Ok(event) => {
                        let mapped = {
                            let mut guard = state.lock().unwrap();
                            guard.translate(&event.event, &event.data)
                        };
                        let mut saw_response = false;
                        for mapped in mapped {
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
        read_secret_from_config(
            &self.secret_config,
            "ANTHROPIC_API_KEY",
            "anthropic_api_key",
        )
    }

    fn request_builder(&self, url: &str, body: &Value, api_key: &str) -> reqwest::RequestBuilder {
        let builder = self
            .http
            .post(url)
            .header("anthropic-version", &self.api_version)
            .header(CONTENT_TYPE, "application/json")
            .json(body);
        match self.auth {
            AnthropicAuth::XApiKey => builder.header("x-api-key", api_key),
            AnthropicAuth::Bearer => builder.header(AUTHORIZATION, format!("Bearer {api_key}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CacheHints, ReasoningConfig};
    use crate::model_standard::InstructionBlock;

    #[test]
    fn provider_config_does_not_require_secret_until_request() {
        let client = AnthropicMessagesClient::from_provider_config(json!({
            "api_key_env": "__PROTEUS_TEST_MISSING_ANTHROPIC_KEY",
            "stream": false
        }))
        .expect("adapter should build without reading env secret");

        assert!(!client.stream_enabled);
    }

    #[test]
    fn provider_config_reads_base_url_from_json_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("anthropic.json");
        std::fs::write(
            &path,
            r#"{ "provider_base_url": "https://anthropic-proxy.example.test" }"#,
        )
        .expect("write secret");

        let client = AnthropicMessagesClient::from_provider_config(json!({
            "base_url_file": path,
            "base_url_json_key": "provider_base_url",
            "api_key_env": "__PROTEUS_TEST_MISSING_ANTHROPIC_KEY",
            "stream": false
        }))
        .expect("adapter should read base_url file");

        assert_eq!(client.base_url, "https://anthropic-proxy.example.test");
    }

    #[test]
    fn request_serializes_reasoning_effort_and_thinking_budget() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("anthropic", "claude-sonnet-4-6"),
            vec![CanonicalMessage::text(MessageRole::User, "solve it")],
        )
        .with_reasoning(
            ReasoningConfig::new(Some("max".to_owned()), true).with_budget_tokens(Some(8192)),
        );

        let body = to_anthropic_request(&request).unwrap();

        assert_eq!(body["output_config"], json!({ "effort": "max" }));
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert_eq!(
            body["thinking"],
            json!({
                "type": "enabled",
                "budget_tokens": 8192,
                "display": "summarized"
            })
        );
    }

    #[test]
    fn request_serializes_summary_as_adaptive_thinking() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("anthropic", "claude-opus-4-7"),
            vec![CanonicalMessage::text(MessageRole::User, "solve it")],
        )
        .with_reasoning(ReasoningConfig::new(Some("xhigh".to_owned()), true));

        let body = to_anthropic_request(&request).unwrap();

        assert_eq!(body["output_config"], json!({ "effort": "xhigh" }));
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert_eq!(
            body["thinking"],
            json!({
                "type": "adaptive",
                "display": "summarized"
            })
        );
    }

    #[test]
    fn request_serializes_prompt_cache_control_when_cache_hints_are_enabled() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("anthropic", "claude-test"),
            vec![CanonicalMessage::text(MessageRole::User, "solve it")],
        )
        .with_instructions(vec![InstructionBlock::new(
            crate::model_standard::InstructionKind::System,
            "stable system prompt",
            100,
        )])
        .with_cache(CacheHints::new(true, true));
        let cache = AnthropicPromptCacheConfig::from_provider_config(&json!({
            "prompt_cache_ttl": "1h"
        }));

        let body = to_anthropic_request_with_cache(&request, &cache).unwrap();

        assert!(body.get("cache_control").is_none());
        assert_eq!(body["system"][0]["type"], "text");
        assert_eq!(body["system"][0]["text"], "stable system prompt");
        assert_eq!(
            body["system"][0]["cache_control"],
            json!({ "type": "ephemeral", "ttl": "1h" })
        );
    }

    #[test]
    fn request_puts_prompt_cache_control_on_last_tool_without_system_prefix() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("anthropic", "claude-test"),
            vec![CanonicalMessage::text(MessageRole::User, "solve it")],
        )
        .with_tools(vec![
            crate::domain::ToolSpec::new(
                "read_file",
                "Read file",
                json!({ "type": "object" }),
                crate::domain::ToolSafety::ReadOnly,
            ),
            crate::domain::ToolSpec::new(
                "write_file",
                "Write file",
                json!({ "type": "object" }),
                crate::domain::ToolSafety::WritesFiles,
            ),
        ])
        .with_cache(CacheHints::new(true, true));
        let cache = AnthropicPromptCacheConfig::from_provider_config(&json!({
            "prompt_cache_ttl": "1h"
        }));

        let body = to_anthropic_request_with_cache(&request, &cache).unwrap();

        assert!(body.get("cache_control").is_none());
        assert!(body["tools"][0].get("cache_control").is_none());
        assert_eq!(
            body["tools"][1]["cache_control"],
            json!({ "type": "ephemeral", "ttl": "1h" })
        );
    }

    #[test]
    fn request_keeps_top_level_cache_control_when_only_context_cache_is_requested() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("anthropic", "claude-test"),
            vec![CanonicalMessage::text(MessageRole::User, "solve it")],
        )
        .with_tools(vec![crate::domain::ToolSpec::new(
            "read_file",
            "Read file",
            json!({ "type": "object" }),
            crate::domain::ToolSafety::ReadOnly,
        )])
        .with_cache(CacheHints::new(false, true));
        let cache = AnthropicPromptCacheConfig::from_provider_config(&json!({
            "prompt_cache_ttl": "1h"
        }));

        let body = to_anthropic_request_with_cache(&request, &cache).unwrap();

        assert_eq!(
            body["cache_control"],
            json!({ "type": "ephemeral", "ttl": "1h" })
        );
        assert!(body["tools"][0].get("cache_control").is_none());
    }

    #[test]
    fn request_keeps_legacy_system_shape_without_cache_hints() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("anthropic", "claude-test"),
            vec![CanonicalMessage::text(MessageRole::User, "solve it")],
        )
        .with_instructions(vec![InstructionBlock::new(
            crate::model_standard::InstructionKind::System,
            "stable system prompt",
            100,
        )]);
        let cache = AnthropicPromptCacheConfig::from_provider_config(&json!({
            "prompt_cache_ttl": "1h"
        }));

        let body = to_anthropic_request_with_cache(&request, &cache).unwrap();

        assert_eq!(body["system"], "stable system prompt");
        assert!(body.get("cache_control").is_none());
    }

    #[test]
    fn request_rejects_freeform_tools_without_fallback() {
        let request = CanonicalModelRequest::new(
            ModelRef::new("anthropic", "claude-test"),
            vec![CanonicalMessage::text(MessageRole::User, "edit")],
        )
        .with_tools(vec![
            ToolSpec::new(
                "apply_patch",
                "Use the `apply_patch` tool to edit files.",
                json!({}),
                crate::domain::ToolSafety::WritesFiles,
            )
            .with_surface(ToolSurface::freeform_lark("start: \"*** Begin Patch\"")),
        ]);

        let error = to_anthropic_request(&request).expect_err("freeform should be unsupported");

        assert!(
            error
                .to_string()
                .contains("anthropic.messages does not support")
        );
    }

    #[test]
    fn request_rejects_freeform_tool_call_history_without_fallback() {
        let call = ToolCall::new(
            "call_1",
            "apply_patch",
            json!({ "input": "*** Begin Patch\n*** Add File: note.txt\n+hello\n*** End Patch" }),
        )
        .with_surface(ToolCallSurface::Freeform);
        let request = CanonicalModelRequest::new(
            ModelRef::new("anthropic", "claude-test"),
            vec![CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call }],
            )],
        );

        let error = to_anthropic_request(&request).expect_err("freeform history is unsupported");

        assert!(
            error
                .to_string()
                .contains("anthropic.messages does not support")
        );
    }

    #[test]
    fn stream_thinking_delta_becomes_reasoning_summary_delta() {
        let mut state = AnthropicStreamState::default();

        assert!(
            state
                .translate(
                    "content_block_start",
                    &json!({
                        "index": 0,
                        "content_block": { "type": "thinking", "thinking": "" }
                    })
                    .to_string()
                )
                .is_empty()
        );
        let events = state.translate(
            "content_block_delta",
            &json!({
                "index": 0,
                "delta": { "type": "thinking_delta", "thinking": "checked constraints" }
            })
            .to_string(),
        );

        assert_eq!(
            events,
            vec![ModelStreamEvent::ReasoningSummaryDelta {
                text: "checked constraints".to_owned()
            }]
        );
        assert!(
            state
                .translate(
                    "content_block_delta",
                    &json!({
                        "index": 0,
                        "delta": { "type": "signature_delta", "signature": "sig" }
                    })
                    .to_string()
                )
                .is_empty()
        );
        let final_events = state.translate("message_stop", &json!({}).to_string());
        match &final_events[0] {
            ModelStreamEvent::Response { response } => {
                assert!(matches!(
                    &response.message.parts[0],
                    ContentPart::Reasoning { text, signature }
                        if text == "checked constraints" && signature.as_deref() == Some("sig")
                ));
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

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

    #[test]
    fn response_usage_includes_cache_details() {
        let response = json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-test",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "hello" }],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 20,
                "cache_creation_input_tokens": 12,
                "cache_read_input_tokens": 34
            }
        });

        let canonical = from_anthropic_response(response).unwrap();
        let usage = canonical.usage.expect("usage");

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.cache_creation_input_tokens, Some(12));
        assert_eq!(usage.cached_input_tokens, Some(34));
    }

    #[test]
    fn sanitize_provider_text_removes_dsml_tool_call_blocks() {
        let text = concat!(
            "before\n",
            "<｜｜DSML｜｜tool_calls>\n",
            "<｜｜DSML｜｜invoke name=\"list_dir\">\n",
            "<｜｜DSML｜｜parameter name=\"path\" string=\"true\">.",
            "</｜｜DSML｜｜parameter>\n",
            "</｜｜DSML｜｜invoke>\n",
            "</｜｜DSML｜｜tool_calls>\n",
            "after"
        );

        let sanitized = sanitize_provider_text(text);

        assert_eq!(sanitized, "before\n\nafter");
        assert!(!sanitized.contains("DSML"));
        assert!(!sanitized.contains("invoke"));
    }

    #[test]
    fn sanitize_provider_text_removes_loose_dsml_tags() {
        let sanitized = sanitize_provider_text(
            "hello <｜｜DSML｜｜invoke name=\"list_dir\">visible</｜｜DSML｜｜invoke> world",
        );

        assert_eq!(sanitized, "hello visible world");
        assert!(!sanitized.contains("DSML"));
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
    fn stream_trace_filters_split_dsml_tool_blocks() {
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
                    "delta": { "type": "text_delta", "text": "before " }
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "<｜｜DSML｜｜tool_calls>\n<｜｜DSML｜｜invoke name=\"list_dir\">" }
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "<｜｜DSML｜｜parameter name=\"path\" string=\"true\">.</｜｜DSML｜｜parameter>" }
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "</｜｜DSML｜｜invoke>\n</｜｜DSML｜｜tool_calls> after" }
                }),
            ),
            (
                "message_delta",
                json!({
                    "delta": { "stop_reason": "end_turn" }
                }),
            ),
            ("message_stop", json!({})),
        ];

        let events = run_trace(&trace);
        let streamed = events
            .iter()
            .filter_map(|event| match event {
                ModelStreamEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();

        assert_eq!(streamed, "before  after");
        assert!(!streamed.contains("DSML"));
        assert!(!streamed.contains("invoke"));

        match events.last().unwrap() {
            ModelStreamEvent::Response { response } => {
                let final_text = response
                    .message
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                assert_eq!(final_text, "before  after");
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn stream_trace_filters_incomplete_dsml_closing_marker() {
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
                    "delta": { "type": "text_delta", "text": "visible " }
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "</｜｜DSML｜｜" }
                }),
            ),
            (
                "message_delta",
                json!({
                    "delta": { "stop_reason": "end_turn" }
                }),
            ),
            ("message_stop", json!({})),
        ];

        let events = run_trace(&trace);
        let streamed = events
            .iter()
            .filter_map(|event| match event {
                ModelStreamEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();

        assert_eq!(streamed, "visible ");
        assert!(!streamed.contains("DSML"));
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
            tool_deltas
                .iter()
                .map(|(_, d)| d.as_str())
                .collect::<Vec<_>>(),
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
