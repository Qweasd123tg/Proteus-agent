//! HTTP request retries from Codex 67cc3c3: model-provider-info and
//! codex-client/retry.rs. The accepted response body/stream is outside this loop.
use std::time::Duration;

use rand::Rng;
use serde_json::Value;

#[derive(Debug, Clone)]
pub(super) struct RequestRetry {
    max_retries: u64,
}

impl RequestRetry {
    pub(super) fn from_config(config: &Value) -> anyhow::Result<Self> {
        let max_retries = config
            .get("request_max_retries")
            .map(|value| {
                value.as_u64().ok_or_else(|| {
                    anyhow::anyhow!("openai request_max_retries must be a non-negative integer")
                })
            })
            .transpose()?
            .unwrap_or(4)
            .min(100);
        Ok(Self { max_retries })
    }

    pub(super) async fn send<F>(&self, mut build: F) -> Result<reqwest::Response, reqwest::Error>
    where
        F: FnMut() -> reqwest::RequestBuilder,
    {
        let mut attempt = 0;
        loop {
            // Building is not a transport operation. Invalid URL/headers/body
            // must fail immediately, before entering the network retry path.
            let (client, request) = build().build_split();
            let response = client.execute(request?).await;
            let retryable = match &response {
                Ok(response) => response.status().is_server_error(),
                Err(_) => true,
            };
            if !retryable || attempt >= self.max_retries {
                // Preserve the final HTTP status and body for typed provider
                // error decoding; do not replace it with a generic retry error.
                return response;
            }
            drop(response);
            attempt += 1;
            // The caller owns the model deadline and cancellation. Dropping
            // this future cancels both the request and this backoff sleep.
            tokio::time::sleep(backoff(attempt)).await;
        }
    }
}

fn backoff(attempt: u64) -> Duration {
    let raw = 200u64.saturating_mul(2u64.saturating_pow(attempt as u32 - 1));
    let jitter: f64 = rand::rng().random_range(0.9..1.1);
    Duration::from_millis((raw as f64 * jitter) as u64)
}

#[cfg(test)]
#[path = "http_retry_tests.rs"]
mod tests;
