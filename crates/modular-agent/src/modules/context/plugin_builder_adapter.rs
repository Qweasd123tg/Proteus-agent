//! Adapter: full `PluginContextBuilder` -> async `ContextBuilder`.

use std::sync::Arc;

use agent_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RResult, RString},
    },
    plugin::{
        ContextBuilderObject, PluginContextBuilderHost, PluginContextBuilderHostError,
        PluginContextBuilderHostMut, PluginContextBuilderHost_TO, PluginContextBuilderInput,
        PluginContextBuilder_TO, PluginContextProviderInput,
    },
};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::runtime::Handle;

use crate::{
    contracts::{ContextBuildInput, ContextBuilder, SearchQuery},
    domain::{ContextBundle, ContextChunk, MemoryQuery},
    modules::RepoAwareContextProvider,
};

pub struct PluginContextBuilderAdapter {
    module_id: String,
    builder: Arc<ContextBuilderObject>,
    config: serde_json::Value,
    context_providers: Vec<(String, Arc<dyn RepoAwareContextProvider>)>,
}

impl PluginContextBuilderAdapter {
    pub fn new(
        module_id: String,
        builder: Arc<ContextBuilderObject>,
        config: serde_json::Value,
        context_providers: Vec<(String, Arc<dyn RepoAwareContextProvider>)>,
    ) -> Self {
        Self {
            module_id,
            builder,
            config,
            context_providers,
        }
    }
}

#[async_trait]
impl ContextBuilder for PluginContextBuilderAdapter {
    async fn build(&self, input: ContextBuildInput) -> Result<ContextBundle> {
        let builder = self.builder.clone();
        let dto = PluginContextBuilderInput {
            task: input.task.clone(),
            config: self.config.clone(),
        };
        let input_json = serde_json::to_string(&dto)?;
        let host_input = input;
        let host_providers = self.context_providers.clone();
        let module_id = self.module_id.clone();
        let handle = Handle::current();

        let output_json = tokio::task::spawn_blocking(move || {
            let mut host = ContextBuilderHost {
                input: host_input,
                context_providers: host_providers,
                handle,
            };
            let mut host_to: PluginContextBuilderHostMut<'_> =
                PluginContextBuilderHost_TO::from_ptr(&mut host, TD_Opaque);
            match PluginContextBuilder_TO::build_json(
                &*builder,
                RString::from(input_json),
                &mut host_to,
            ) {
                RResult::ROk(output_json) => Ok(output_json.into_string()),
                RResult::RErr(error) => Err(anyhow!(
                    "context builder plugin '{module_id}' error: {}",
                    error.message
                )),
            }
        })
        .await
        .map_err(|join_err| anyhow!("context builder plugin join error: {join_err}"))??;

        serde_json::from_str(&output_json)
            .map_err(|error| anyhow!("context builder plugin returned invalid ContextBundle JSON: {error}"))
    }
}

struct ContextBuilderHost {
    input: ContextBuildInput,
    context_providers: Vec<(String, Arc<dyn RepoAwareContextProvider>)>,
    handle: Handle,
}

impl ContextBuilderHost {
    fn block_on_json<T, F>(&self, future: F) -> RResult<RString, PluginContextBuilderHostError>
    where
        T: serde::Serialize,
        F: std::future::Future<Output = Result<T>>,
    {
        match self.handle.block_on(future) {
            Ok(value) => match serde_json::to_string(&value) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => {
                    RResult::RErr(PluginContextBuilderHostError::new(error.to_string()))
                }
            },
            Err(error) => RResult::RErr(PluginContextBuilderHostError::new(format!("{error:#}"))),
        }
    }
}

impl PluginContextBuilderHost for ContextBuilderHost {
    fn search_json(&self, query_json: RString) -> RResult<RString, PluginContextBuilderHostError> {
        let query: SearchQuery = match serde_json::from_str(query_json.as_str()) {
            Ok(query) => query,
            Err(error) => {
                return RResult::RErr(PluginContextBuilderHostError::new(error.to_string()));
            }
        };
        let search = self.input.search.clone();
        self.block_on_json(async move { search.search(query).await })
    }

    fn recall_memory_json(
        &self,
        query_json: RString,
    ) -> RResult<RString, PluginContextBuilderHostError> {
        let query: MemoryQuery = match serde_json::from_str(query_json.as_str()) {
            Ok(query) => query,
            Err(error) => {
                return RResult::RErr(PluginContextBuilderHostError::new(error.to_string()));
            }
        };
        let memory = self.input.memory.clone();
        self.block_on_json(async move { memory.recall(query).await })
    }

    fn context_provider_json(
        &self,
        provider_id: RString,
        input_json: RString,
    ) -> RResult<RString, PluginContextBuilderHostError> {
        let provider_id = provider_id.into_string();
        let provider_input: PluginContextProviderInput =
            match serde_json::from_str(input_json.as_str()) {
                Ok(input) => input,
                Err(error) => {
                    return RResult::RErr(PluginContextBuilderHostError::new(error.to_string()));
                }
            };
        let Some((_, provider)) = self
            .context_providers
            .iter()
            .find(|(id, _)| id == &provider_id)
        else {
            return RResult::RErr(PluginContextBuilderHostError::new(format!(
                "unknown context provider: {provider_id}"
            )));
        };
        let provider = provider.clone();
        let input = ContextBuildInput {
            task: provider_input.task,
            search: self.input.search.clone(),
            memory: self.input.memory.clone(),
        };
        self.block_on_json::<Vec<ContextChunk>, _>(async move { provider.provide(&input).await })
    }
}
