use std::{path::Path, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use proteus_module_protocol::{
    HostRequestDispatcher, ProcessModuleHostRequest, ProcessModuleRpcError,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::runtime::Handle;

use crate::{
    contracts::{
        CONTEXT_HOST_PROVIDER_METHOD, CONTEXT_HOST_RECALL_MEMORY_METHOD,
        CONTEXT_HOST_SEARCH_METHOD, ContextBuildInput, ContextBuilder,
        PROCESS_CONTEXT_BUILD_METHOD, PROCESS_CONTEXT_CONTRACT_VERSION,
        PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION, PROCESS_CONTEXT_PROVIDER_METHOD,
        ProcessContextChunksResponse, ProcessContextInput, ProcessContextProviderInput,
        ProcessContextRecallInput, ProcessContextResponse, ProcessContextSearchInput,
    },
    core::RepoAwareContextProvider,
    domain::{ContextBundle, ContextChunk},
};

use super::{ProcessExportClient, ProcessExportConfig};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const HOST_CALLBACK_ERROR: i64 = -32_100;

pub struct ProcessContextBuilder {
    client: Arc<ProcessExportClient>,
    providers: Vec<(String, Arc<dyn RepoAwareContextProvider>)>,
}

impl ProcessContextBuilder {
    pub fn new(
        config: ProcessExportConfig,
        workspace: &Path,
        providers: Vec<(String, Arc<dyn RepoAwareContextProvider>)>,
    ) -> Result<Self> {
        Ok(Self {
            client: Arc::new(ProcessExportClient::connect(
                "context",
                PROCESS_CONTEXT_CONTRACT_VERSION,
                config,
                workspace,
                DEFAULT_TIMEOUT_MS,
            )?),
            providers,
        })
    }
}

#[async_trait]
impl ContextBuilder for ProcessContextBuilder {
    async fn build(&self, input: ContextBuildInput) -> Result<ContextBundle> {
        let request = ProcessContextInput {
            task: input.task.clone(),
        };
        let client = Arc::clone(&self.client);
        let dispatcher: Arc<dyn HostRequestDispatcher> = Arc::new(ContextDispatcher {
            input,
            providers: self.providers.clone(),
            handle: Handle::current(),
        });
        tokio::task::spawn_blocking(move || {
            let response: ProcessContextResponse = client.invoke_with_dispatcher(
                PROCESS_CONTEXT_BUILD_METHOD,
                &request,
                dispatcher,
                || false,
            )?;
            Ok(response.result)
        })
        .await
        .map_err(|error| anyhow::anyhow!("process context join error: {error}"))?
    }
}

struct ContextDispatcher {
    input: ContextBuildInput,
    providers: Vec<(String, Arc<dyn RepoAwareContextProvider>)>,
    handle: Handle,
}

impl HostRequestDispatcher for ContextDispatcher {
    fn dispatch(&self, request: ProcessModuleHostRequest) -> Result<Value, ProcessModuleRpcError> {
        match request.method.as_str() {
            CONTEXT_HOST_SEARCH_METHOD => {
                let input = decode::<ProcessContextSearchInput>(request.params, &request.method)?;
                let search = Arc::clone(&self.input.search);
                host_result(
                    self.handle
                        .block_on(async move { search.search(input.query).await }),
                    &request.method,
                )
            }
            CONTEXT_HOST_RECALL_MEMORY_METHOD => {
                let input = decode::<ProcessContextRecallInput>(request.params, &request.method)?;
                let memory = Arc::clone(&self.input.memory);
                host_result(
                    self.handle
                        .block_on(async move { memory.recall(input.query).await }),
                    &request.method,
                )
            }
            CONTEXT_HOST_PROVIDER_METHOD => {
                let input = decode::<ProcessContextProviderInput>(request.params, &request.method)?;
                let provider = self
                    .providers
                    .iter()
                    .find(|(id, _)| id == &input.provider_id)
                    .map(|(_, provider)| Arc::clone(provider))
                    .ok_or_else(|| {
                        ProcessModuleRpcError::new(
                            -32602,
                            format!("unknown context provider: {}", input.provider_id),
                        )
                    })?;
                let provider_input = ContextBuildInput {
                    task: input.task,
                    search: Arc::clone(&self.input.search),
                    memory: Arc::clone(&self.input.memory),
                };
                host_result(
                    self.handle
                        .block_on(async move { provider.provide(&provider_input).await }),
                    &request.method,
                )
            }
            method => Err(ProcessModuleRpcError::new(
                -32601,
                format!("context host method is not implemented: {method}"),
            )),
        }
    }
}

pub struct ProcessContextProvider {
    provider_id: String,
    client: Arc<ProcessExportClient>,
}

impl ProcessContextProvider {
    pub fn new(config: ProcessExportConfig, workspace: &Path) -> Result<Self> {
        let provider_id = config.module_id().to_owned();
        Ok(Self {
            provider_id,
            client: Arc::new(ProcessExportClient::connect(
                "context_provider",
                PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION,
                config,
                workspace,
                DEFAULT_TIMEOUT_MS,
            )?),
        })
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }
}

#[async_trait]
impl RepoAwareContextProvider for ProcessContextProvider {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let request = ProcessContextProviderInput {
            provider_id: self.provider_id.clone(),
            task: input.task.clone(),
            metadata: Value::Null,
        };
        let client = Arc::clone(&self.client);
        tokio::task::spawn_blocking(move || {
            let response: ProcessContextChunksResponse =
                client.invoke(PROCESS_CONTEXT_PROVIDER_METHOD, &request)?;
            Ok(response.result)
        })
        .await
        .map_err(|error| anyhow::anyhow!("process context provider join error: {error}"))?
    }
}

fn decode<T: DeserializeOwned>(params: Value, method: &str) -> Result<T, ProcessModuleRpcError> {
    serde_json::from_value(params).map_err(|error| {
        ProcessModuleRpcError::new(-32602, format!("invalid {method} params: {error}"))
    })
}

fn host_result<T: Serialize>(
    result: Result<T>,
    method: &str,
) -> Result<Value, ProcessModuleRpcError> {
    let value = result.map_err(|error| {
        ProcessModuleRpcError::new(HOST_CALLBACK_ERROR, format!("{method} failed: {error:#}"))
    })?;
    serde_json::to_value(value).map_err(|error| {
        ProcessModuleRpcError::new(
            -32603,
            format!("failed to serialize {method} response: {error}"),
        )
    })
}
