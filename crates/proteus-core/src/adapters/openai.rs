use std::sync::Arc;

use anyhow::Result;
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
    domain::{ToolCall, ToolCallSurface, ToolChoice, ToolSpec, ToolSurface},
    model_standard::{CanonicalMessage, ContentPart, FinishReason, MessageRole},
};

mod hosted_tools;
mod model_profile;
mod request;
mod response;
mod stream;
#[cfg(test)]
mod tests;

use model_profile::OpenAiModelProfile;
#[cfg(test)]
use request::to_openai_request;
use request::to_openai_request_with_cache;
use response::from_openai_response;
use stream::{finalize_completed_event, translate_sse_event};

#[derive(Debug, Clone)]
pub struct OpenAiResponsesClient {
    http: reqwest::Client,
    secret_config: Value,
    base_url: String,
    /// Включает SSE-стрим на `/responses`. Управляется через поле
    /// `stream` в provider config. Provider profiles по умолчанию включают
    /// streaming; `stream = false` оставляет non-stream fallback.
    stream_enabled: bool,
    /// Explicit diagnostic recovery mode for endpoints that corrupt SSE bodies.
    /// Strict Codex-shaped profiles leave this disabled: replaying a full
    /// inference request after partial output can duplicate side effects.
    stream_error_fallback: bool,
    /// Потолок контекстного окна (`max_input_tokens` в provider config).
    /// Сообщается в capabilities и питает индикатор заполнения контекста.
    max_input_tokens: Option<u32>,
    prompt_cache: OpenAiPromptCacheConfig,
    model_profile: OpenAiModelProfile,
}

#[derive(Debug, Clone, Default)]
struct OpenAiPromptCacheConfig {
    enabled: bool,
    key: Option<String>,
    retention: Option<String>,
}

impl OpenAiResponsesClient {
    pub fn from_provider_config(config: Value) -> Result<Self> {
        let base_url = read_config_string_or_default(
            &config,
            "base_url",
            "https://api.openai.com/v1",
            "base_url",
        )?
        .trim_end_matches('/')
        .to_owned();
        let stream_enabled = config
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let stream_error_fallback = config
            .get("stream_error_fallback")
            .map(|value| {
                value.as_bool().ok_or_else(|| {
                    anyhow::anyhow!("openai stream_error_fallback must be a boolean")
                })
            })
            .transpose()?
            .unwrap_or(false);
        let max_input_tokens = config
            .get("max_input_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0);
        let prompt_cache = OpenAiPromptCacheConfig::from_provider_config(&config);
        let model_profile = OpenAiModelProfile::from_provider_config(&config)?;
        let http1_only = config
            .get("http1_only")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| anyhow::anyhow!("openai http1_only must be a boolean"))
            })
            .transpose()?
            .unwrap_or(false);
        let mut http = reqwest::Client::builder();
        if http1_only {
            http = http.http1_only();
        }

        Ok(Self {
            http: http.build()?,
            secret_config: config,
            base_url,
            stream_enabled,
            stream_error_fallback,
            max_input_tokens,
            prompt_cache,
            model_profile,
        })
    }
}

pub fn build_openai_responses_adapter(config: Value) -> Result<Arc<dyn Model>> {
    Ok(Arc::new(OpenAiResponsesClient::from_provider_config(
        config,
    )?))
}

impl OpenAiPromptCacheConfig {
    fn from_provider_config(config: &Value) -> Self {
        Self {
            enabled: config
                .get("prompt_cache")
                .or_else(|| config.get("prompt_caching"))
                .and_then(Value::as_bool)
                .unwrap_or(true),
            key: non_empty_config_string(config, "prompt_cache_key"),
            retention: non_empty_config_string(config, "prompt_cache_retention"),
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

#[async_trait]
impl Model for OpenAiResponsesClient {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "openai.responses".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        self.model_profile.capabilities(self.max_input_tokens)
    }

    fn provider_hosted_tools(&self, _model: &ModelRef) -> Vec<crate::domain::ToolSpec> {
        self.model_profile.hosted_tools.specs()
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

impl OpenAiResponsesClient {
    async fn complete_response(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse> {
        let body = to_openai_request_with_cache(&request, &self.prompt_cache, &self.model_profile)?;
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
        let mut body =
            to_openai_request_with_cache(&request, &self.prompt_cache, &self.model_profile)?;
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
            // Накапливаем финализированные output-item'ы из response.output_item.done.
            // Некоторые OpenAI-совместимые прокси отдают response.completed с пустым
            // output, хотя сами item'ы (message/function_call) уже были доставлены
            // через output_item.done. Без этого ход теряется как <empty model response>.
            let mut completed_items: Vec<Value> = Vec::new();
            let mut streamed_text = String::new();
            let mut saw_terminal_event = false;
            while let Some(chunk) = sse.next().await {
                match chunk {
                    Ok(event) => {
                        if event.event == "response.output_item.done"
                            && let Ok(parsed) = serde_json::from_str::<Value>(&event.data)
                            && let Some(item) = parsed.get("item")
                        {
                            completed_items.push(item.clone());
                        }
                        let mapped = if event.event == "response.completed" {
                            finalize_completed_event(&event.data, &completed_items, &streamed_text)
                        } else {
                            translate_sse_event(&event.event, &event.data)
                        };
                        for mapped in mapped {
                            if let ModelStreamEvent::TextDelta { text } = &mapped {
                                streamed_text.push_str(text);
                            }
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
                        if client.stream_error_fallback {
                            match client.complete_response(fallback_request).await {
                                Ok(response) => yield Ok(ModelStreamEvent::Response { response }),
                                Err(fallback_error) => yield Ok(ModelStreamEvent::Error {
                                    message: format!(
                                        "sse transport error: {error}; non-stream fallback failed: {fallback_error}"
                                    ),
                                }),
                            }
                        } else {
                            yield Ok(ModelStreamEvent::Error {
                                message: format!("sse transport error: {error}"),
                            });
                        }
                        saw_terminal_event = true;
                        break;
                    }
                }
            }
            if !saw_terminal_event {
                yield Ok(ModelStreamEvent::Error {
                    message: "openai responses stream ended without a terminal event".to_owned(),
                });
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
