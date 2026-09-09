//! Responses authentication and transport differences stay inside model-pack.
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::{
    StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use serde_json::{Value, json};

use super::{OpenAiResponsesClient, errors::ensure_success, to_openai_request_with_cache};
use crate::{
    adapters::{
        codex_auth::{CODEX_BASE_URL, CodexAuth, validate_endpoint},
        secrets::read_secret_from_config,
    },
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, ModelFailure, ModelStreamEvent,
    },
};

impl OpenAiResponsesClient {
    pub(super) fn from_codex_config(mut config: Value) -> Result<Self> {
        super::codex_config::validate(&config)?;
        let object = config
            .as_object_mut()
            .context("openai_codex config must be an object")?;
        if object
            .get("stream_error_fallback")
            .is_some_and(|value| value != &json!(false))
        {
            bail!("openai_codex does not support stream_error_fallback");
        }
        object
            .entry("base_url")
            .or_insert_with(|| json!(CODEX_BASE_URL));
        let base_url = validate_endpoint(
            object["base_url"]
                .as_str()
                .context("openai_codex base_url must be a string")?,
        )?;
        let auth = CodexAuth::from_config(&config)?;
        let mut client = Self::from_provider_config(config)?;
        let mut http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
        if client.secret_config.get("http1_only") == Some(&json!(true)) {
            http = http.http1_only();
        }
        client.http = http.build()?;
        client.base_url = base_url;
        client.codex_auth = Some(auth);
        Ok(client)
    }

    pub(super) fn request_body(
        &self,
        request: &CanonicalModelRequest,
        stream: bool,
    ) -> Result<Value> {
        let mut body =
            to_openai_request_with_cache(request, &self.prompt_cache, &self.model_profile)?;
        if stream {
            body["stream"] = json!(true);
        }
        if self.codex_auth.is_some() {
            // Codex's subscription Responses endpoint is streaming-only and
            // doesn't accept max_output_tokens. Match its request surface,
            // including internal complete calls (collected from the same SSE).
            body["stream"] = json!(true);
            body.as_object_mut().unwrap().remove("max_output_tokens");
            body.as_object_mut()
                .unwrap()
                .entry("instructions")
                .or_insert_with(|| json!(""));
        }
        Ok(body)
    }

    pub(super) async fn send_response(&self, body: &Value) -> Result<reqwest::Response> {
        let url = format!("{}/responses", self.base_url);
        let Some(auth) = &self.codex_auth else {
            let key =
                read_secret_from_config(&self.secret_config, "OPENAI_API_KEY", "openai_api_key")?;
            let mut bearer = HeaderValue::from_str(&format!("Bearer {key}"))?;
            bearer.set_sensitive(true);
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, bearer);
            return ensure_success(
                self.request_retry
                    .send(|| self.request_builder(&url, body, &headers))
                    .await?,
            )
            .await;
        };
        let access = auth.access(None).await?;
        let mut response = self
            .request_retry
            .send(|| self.request_builder(&url, body, &access.headers))
            .await?;
        // A rejected request has no accepted model output. Refresh once; all
        // 429s and other 4xx retain the ordinary error path, without API fallback.
        if response.status() == StatusCode::UNAUTHORIZED {
            drop(response);
            let access = auth.access(Some(access.token)).await?;
            response = self
                .request_retry
                .send(|| self.request_builder(&url, body, &access.headers))
                .await?;
        }
        ensure_success(response).await
    }

    fn request_builder(
        &self,
        url: &str,
        body: &Value,
        headers: &HeaderMap,
    ) -> reqwest::RequestBuilder {
        self.http
            .post(url)
            .headers(headers.clone())
            .header(CONTENT_TYPE, "application/json")
            .json(body)
    }

    pub(super) async fn collect_codex_response(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse> {
        let mut stream = self.stream_response(request).await?;
        let mut completed = Vec::new();
        while let Some(event) = stream.next().await {
            match event? {
                ModelStreamEvent::Response { response } => return Ok(response),
                ModelStreamEvent::MessageCompleted { message } => completed.push(message),
                ModelStreamEvent::Error { mut failure } => {
                    failure.completed_messages = completed;
                    return Err(failure.into());
                }
                _ => {}
            }
        }
        Err(ModelFailure::other("ChatGPT response ended without a terminal event").into())
    }
}
