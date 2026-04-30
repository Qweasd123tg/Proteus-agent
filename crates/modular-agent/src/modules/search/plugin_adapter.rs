//! Адаптер: `SearchBackendObject` → `Arc<dyn SearchBackend>`.
//!
//! `SearchBackend` в ядре async, `PluginSearchBackend` — sync (sabi_trait
//! не поддерживает async). Мост через `tokio::task::spawn_blocking`, DTO
//! через JSON. Эталон — `modules/patch/plugin_adapter.rs`.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;

use agent_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{PluginSearchBackend_TO, SearchBackendObject},
};

use crate::{
    contracts::{SearchBackend, SearchQuery},
    domain::ContextChunk,
};

pub struct PluginSearchAdapter {
    inner: Arc<SearchBackendObject>,
}

impl PluginSearchAdapter {
    pub fn new(backend: SearchBackendObject) -> Self {
        Self {
            inner: Arc::new(backend),
        }
    }
}

#[async_trait]
impl SearchBackend for PluginSearchAdapter {
    async fn search(&self, query: SearchQuery) -> Result<Vec<ContextChunk>> {
        let query_json = serde_json::to_string(&query)
            .with_context(|| "plugin search: serialize SearchQuery failed")?;
        let inner = self.inner.clone();

        let result_json = tokio::task::spawn_blocking(move || {
            let q = RString::from(query_json);
            match PluginSearchBackend_TO::search_json(&*inner, q) {
                RResult::ROk(s) => Ok(s.into_string()),
                RResult::RErr(err) => Err(anyhow!("plugin search error: {}", err.message)),
            }
        })
        .await
        .map_err(|join_err| anyhow!("plugin search join error: {join_err}"))??;

        let chunks: Vec<ContextChunk> = serde_json::from_str(&result_json)
            .with_context(|| "plugin search returned invalid result JSON")?;
        Ok(chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_contracts::{
        abi_stable::{sabi_trait::TD_Opaque, std_types::RResult::ROk},
        plugin::{PluginSearchBackend, PluginSearchBackend_TO, PluginSearchError},
    };

    struct StaticBackend;
    impl PluginSearchBackend for StaticBackend {
        fn search_json(&self, _query: RString) -> RResult<RString, PluginSearchError> {
            let chunks = vec![ContextChunk::new("plugin:static", "hello").with_score(1.0)];
            ROk(serde_json::to_string(&chunks).unwrap().into())
        }
    }

    struct FailBackend;
    impl PluginSearchBackend for FailBackend {
        fn search_json(&self, _query: RString) -> RResult<RString, PluginSearchError> {
            RResult::RErr(PluginSearchError::new("backend exploded"))
        }
    }

    struct BrokenJsonBackend;
    impl PluginSearchBackend for BrokenJsonBackend {
        fn search_json(&self, _query: RString) -> RResult<RString, PluginSearchError> {
            ROk(RString::from("not json"))
        }
    }

    fn wrap(backend: impl PluginSearchBackend + 'static) -> PluginSearchAdapter {
        let obj = PluginSearchBackend_TO::from_value(backend, TD_Opaque);
        PluginSearchAdapter::new(obj)
    }

    fn make_query() -> SearchQuery {
        SearchQuery::new("needle", std::path::PathBuf::from("/tmp"), 10)
    }

    #[tokio::test]
    async fn plugin_success_round_trip() {
        let adapter = wrap(StaticBackend);
        let chunks = adapter.search(make_query()).await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source, "plugin:static");
        assert_eq!(chunks[0].content, "hello");
    }

    #[tokio::test]
    async fn plugin_rerror_propagates_as_anyhow() {
        let adapter = wrap(FailBackend);
        let err = adapter.search(make_query()).await.unwrap_err();
        assert!(err.to_string().contains("backend exploded"), "{err}");
    }

    #[tokio::test]
    async fn invalid_json_propagates_as_anyhow() {
        let adapter = wrap(BrokenJsonBackend);
        let err = adapter.search(make_query()).await.unwrap_err();
        assert!(err.to_string().contains("invalid result JSON"), "{err}");
    }
}
