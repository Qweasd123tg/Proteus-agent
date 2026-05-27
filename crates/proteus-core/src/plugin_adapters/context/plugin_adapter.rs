use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;

use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{ContextProviderObject, PluginContextProvider_TO, PluginContextProviderInput},
};

use crate::{contracts::ContextBuildInput, core::RepoAwareContextProvider, domain::ContextChunk};

pub struct PluginContextProviderAdapter {
    provider_id: String,
    inner: Arc<ContextProviderObject>,
}

impl PluginContextProviderAdapter {
    pub fn new(provider_id: impl Into<String>, provider: ContextProviderObject) -> Self {
        Self {
            provider_id: provider_id.into(),
            inner: Arc::new(provider),
        }
    }
}

#[async_trait]
impl RepoAwareContextProvider for PluginContextProviderAdapter {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let dto = PluginContextProviderInput {
            provider_id: self.provider_id.clone(),
            task: input.task.clone(),
            metadata: serde_json::Value::Null,
        };
        let input_json = serde_json::to_string(&dto)
            .with_context(|| "plugin context provider: serialize input failed")?;
        let inner = self.inner.clone();

        let result_json = tokio::task::spawn_blocking(move || {
            match PluginContextProvider_TO::provide_json(&*inner, RString::from(input_json)) {
                RResult::ROk(s) => Ok(s.into_string()),
                RResult::RErr(err) => {
                    Err(anyhow!("plugin context provider error: {}", err.message))
                }
            }
        })
        .await
        .map_err(|join_err| anyhow!("plugin context provider join error: {join_err}"))??;

        serde_json::from_str(&result_json)
            .with_context(|| "plugin context provider returned invalid result JSON")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::{
        abi_stable::{sabi_trait::TD_Opaque, std_types::RResult::ROk},
        plugin::{PluginContextError, PluginContextProvider, PluginContextProvider_TO},
    };

    use crate::{
        contracts::{MemoryStore, SearchBackend},
        domain::{AgentTask, ContextChunk},
        stubs::{NoMemory, NullSearch},
    };

    struct StaticProvider;

    impl PluginContextProvider for StaticProvider {
        fn provide_json(&self, _input_json: RString) -> RResult<RString, PluginContextError> {
            let chunks = vec![ContextChunk::new("plugin:context", "from plugin").with_score(0.9)];
            ROk(serde_json::to_string(&chunks).unwrap().into())
        }
    }

    #[tokio::test]
    async fn plugin_context_provider_round_trips_chunks() {
        let obj = PluginContextProvider_TO::from_value(StaticProvider, TD_Opaque);
        let adapter = PluginContextProviderAdapter::new("static", obj);
        let input = ContextBuildInput {
            task: AgentTask::new("task", std::path::PathBuf::from("/tmp")),
            search: Arc::new(NullSearch) as Arc<dyn SearchBackend>,
            memory: Arc::new(NoMemory) as Arc<dyn MemoryStore>,
        };

        let chunks = adapter.provide(&input).await.unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source, "plugin:context");
        assert_eq!(chunks[0].content, "from plugin");
    }
}
