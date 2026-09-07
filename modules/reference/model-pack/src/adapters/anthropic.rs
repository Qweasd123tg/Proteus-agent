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
    contracts::{Model, ModelEventStream},
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

pub fn build_anthropic_messages_adapter(config: Value) -> Result<Arc<dyn Model>> {
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
impl Model for AnthropicMessagesClient {
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
            let mut saw_terminal_event = false;
            while let Some(chunk) = sse.next().await {
                match chunk {
                    Ok(event) => {
                        let mapped = {
                            let mut guard = state.lock().unwrap();
                            guard.translate(&event.event, &event.data)
                        };
                        for mapped in mapped {
                            if matches!(
                                mapped,
                                ModelStreamEvent::Response { .. } | ModelStreamEvent::Error { .. }
                            ) {
                                saw_terminal_event = true;
                            }
                            yield Ok(mapped);
                        }
                        if saw_terminal_event {
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
                        saw_terminal_event = true;
                        break;
                    }
                }
            }
            if !saw_terminal_event {
                yield Ok(ModelStreamEvent::Error {
                    message: "anthropic messages stream ended without a terminal event".to_owned(),
                });
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
mod tests;
