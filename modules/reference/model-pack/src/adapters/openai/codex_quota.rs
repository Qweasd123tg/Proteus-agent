//! ChatGPT quota HTTP/DTO details stay inside the provider implementation.
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use proteus_contracts::contracts::{
    ModelQuotaBucket, ModelQuotaCredits, ModelQuotaSnapshot, ModelQuotaWindow,
};
use serde::Deserialize;

use super::OpenAiResponsesClient;

pub(super) const DEFAULT_QUOTA_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
pub(super) type QuotaCache = tokio::sync::Mutex<Option<(Instant, ModelQuotaSnapshot)>>;
const CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct QuotaResponse {
    plan_type: String,
    rate_limit: Option<RateLimit>,
    additional_rate_limits: Option<Vec<AdditionalLimit>>,
    credits: Option<Credits>,
}

#[derive(Deserialize)]
struct RateLimit {
    allowed: bool,
    limit_reached: bool,
    primary_window: Option<Window>,
    secondary_window: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    used_percent: f64,
    limit_window_seconds: u64,
    reset_at: u64,
}

#[derive(Deserialize)]
struct AdditionalLimit {
    metered_feature: String,
    limit_name: String,
    rate_limit: Option<RateLimit>,
}

#[derive(Deserialize)]
struct Credits {
    has_credits: bool,
    unlimited: bool,
    balance: Option<String>,
}

fn bucket(id: String, name: Option<String>, limit: Option<RateLimit>) -> ModelQuotaBucket {
    let mut result = ModelQuotaBucket {
        id,
        name,
        allowed: None,
        limit_reached: None,
        windows: vec![],
    };
    if let Some(limit) = limit {
        result.allowed = Some(limit.allowed);
        result.limit_reached = Some(limit.limit_reached);
        for (id, window) in [
            ("primary", limit.primary_window),
            ("secondary", limit.secondary_window),
        ] {
            if let Some(window) = window {
                result.windows.push(ModelQuotaWindow {
                    id: id.into(),
                    used_percent: window.used_percent,
                    duration_seconds: Some(window.limit_window_seconds),
                    resets_at: Some(window.reset_at),
                });
            }
        }
    }
    result
}

impl QuotaResponse {
    fn into_snapshot(self) -> Result<ModelQuotaSnapshot> {
        let mut buckets = vec![bucket(
            "codex".into(),
            Some("Codex".into()),
            self.rate_limit,
        )];
        buckets.extend(
            self.additional_rate_limits
                .into_iter()
                .flatten()
                .map(|limit| {
                    bucket(
                        limit.metered_feature,
                        Some(limit.limit_name),
                        limit.rate_limit,
                    )
                }),
        );
        let snapshot = ModelQuotaSnapshot {
            observed_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            plan: Some(self.plan_type),
            buckets,
            credits: self.credits.map(|credits| ModelQuotaCredits {
                available: credits.has_credits,
                unlimited: credits.unlimited,
                balance: credits.balance,
            }),
        };
        snapshot.validate().map_err(anyhow::Error::msg)?;
        Ok(snapshot)
    }
}

impl OpenAiResponsesClient {
    pub(super) async fn codex_quota(&self) -> Result<Option<ModelQuotaSnapshot>> {
        let Some(auth) = &self.codex_auth else {
            return Ok(None);
        };
        let url = self
            .quota_url
            .as_ref()
            .context("ChatGPT quota endpoint is not configured")?;
        tokio::time::timeout(Duration::from_secs(30), async {
            // Serialize readers of this configured export; expiry never serves stale data.
            let mut cache = self.quota_cache.lock().await;
            if let Some((fetched, snapshot)) = &*cache {
                if fetched.elapsed() < CACHE_TTL {
                    return Ok(Some(snapshot.clone()));
                }
            }
            let mut access = auth.access(None).await?;
            for attempt in 0..2 {
                let response = self
                    .http
                    .get(url)
                    .headers(access.headers.clone())
                    .send()
                    .await
                    .context("ChatGPT quota request failed")?;
                if response.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                    access = auth.access(Some(access.token)).await?;
                    continue;
                }
                if !response.status().is_success() {
                    bail!("ChatGPT quota returned HTTP {}", response.status());
                }
                let response: QuotaResponse = response
                    .json()
                    .await
                    .map_err(|_| anyhow::anyhow!("invalid ChatGPT quota response"))?;
                let snapshot = response.into_snapshot()?;
                *cache = Some((Instant::now(), snapshot.clone()));
                return Ok(Some(snapshot));
            }
            unreachable!()
        })
        .await
        .context("ChatGPT quota timed out")?
    }
}
