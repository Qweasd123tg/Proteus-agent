//! Codex /models metadata stays inside the provider; no remote instructions
//! or capabilities are imported into the configured Proteus assembly.
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use proteus_contracts::contracts::{ModelCatalog, ModelCatalogEntry};
use serde::Deserialize;

use super::OpenAiResponsesClient;

pub(super) type CatalogCache = tokio::sync::Mutex<Option<(Instant, ModelCatalog)>>;
const CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Deserialize)]
struct ModelsResponse {
    models: Vec<RemoteModel>,
}

#[derive(Deserialize)]
struct RemoteModel {
    slug: String,
    display_name: String,
    description: Option<String>,
    visibility: String,
    priority: i64,
    default_reasoning_level: Option<String>,
    supported_reasoning_levels: Vec<RemoteEffort>,
}

#[derive(Deserialize)]
struct RemoteEffort {
    effort: String,
}

impl OpenAiResponsesClient {
    pub(super) async fn codex_catalog(&self) -> Result<Option<ModelCatalog>> {
        let Some(auth) = &self.codex_auth else {
            return Ok(None);
        };
        let mut cache = self.catalog_cache.lock().await;
        if let Some((fetched, catalog)) = &*cache {
            if fetched.elapsed() < CACHE_TTL {
                return Ok(Some(catalog.clone()));
            }
        }
        let catalog = tokio::time::timeout(Duration::from_secs(30), async {
            let mut access = auth.access(None).await?;
            // Like upstream, send a semantic client version to the catalog endpoint.
            let version = env!("CARGO_PKG_VERSION").split('-').next().unwrap();
            for attempt in 0..2 {
                let response = self
                    .http
                    .get(format!("{}/models", self.base_url))
                    .query(&[("client_version", version)])
                    .headers(access.headers.clone())
                    .send()
                    .await
                    .context("ChatGPT model catalog request failed")?;
                if response.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                    access = auth.access(Some(access.token)).await?;
                    continue;
                }
                if !response.status().is_success() {
                    bail!("ChatGPT model catalog returned HTTP {}", response.status());
                }
                let mut response: ModelsResponse = response
                    .json()
                    .await
                    .context("invalid ChatGPT model catalog response")?;
                response.models.sort_by_key(|model| model.priority);
                let catalog = ModelCatalog {
                    models: response
                        .models
                        .into_iter()
                        .map(|model| ModelCatalogEntry {
                            id: model.slug,
                            display_name: model.display_name,
                            description: model.description,
                            hidden: model.visibility != "list",
                            reasoning_efforts: model
                                .supported_reasoning_levels
                                .into_iter()
                                .map(|level| level.effort)
                                .collect(),
                            default_reasoning_effort: model.default_reasoning_level,
                        })
                        .collect(),
                };
                catalog.validate().map_err(anyhow::Error::msg)?;
                return Ok(catalog);
            }
            unreachable!()
        })
        .await
        .context("ChatGPT model catalog timed out")??;
        *cache = Some((Instant::now(), catalog.clone()));
        Ok(Some(catalog))
    }
}
