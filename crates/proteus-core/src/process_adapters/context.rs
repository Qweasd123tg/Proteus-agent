use std::{path::Path, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use proteus_module_protocol::{
    ProcessModuleRpcError,
    v3::{AsyncHostRequestDispatcher, ComponentHostRequest, HostRequestFuture},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

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
        let dispatcher: Arc<dyn AsyncHostRequestDispatcher> = Arc::new(ContextDispatcher {
            input,
            providers: self.providers.clone(),
        });
        let response: ProcessContextResponse = self
            .client
            .invoke_with_dispatcher(PROCESS_CONTEXT_BUILD_METHOD, &request, dispatcher)
            .await?;
        Ok(response.result)
    }
}

struct ContextDispatcher {
    input: ContextBuildInput,
    providers: Vec<(String, Arc<dyn RepoAwareContextProvider>)>,
}

impl AsyncHostRequestDispatcher for ContextDispatcher {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        let method = request.method;
        match method.as_str() {
            CONTEXT_HOST_SEARCH_METHOD => {
                let input = match decode::<ProcessContextSearchInput>(request.params, &method) {
                    Ok(input) => input,
                    Err(error) => return Box::pin(async move { Err(error) }),
                };
                let search = Arc::clone(&self.input.search);
                Box::pin(async move { host_result(search.search(input.query).await, &method) })
            }
            CONTEXT_HOST_RECALL_MEMORY_METHOD => {
                let input = match decode::<ProcessContextRecallInput>(request.params, &method) {
                    Ok(input) => input,
                    Err(error) => return Box::pin(async move { Err(error) }),
                };
                let memory = Arc::clone(&self.input.memory);
                Box::pin(async move { host_result(memory.recall(input.query).await, &method) })
            }
            CONTEXT_HOST_PROVIDER_METHOD => {
                let input = match decode::<ProcessContextProviderInput>(request.params, &method) {
                    Ok(input) => input,
                    Err(error) => return Box::pin(async move { Err(error) }),
                };
                let Some(provider) = self
                    .providers
                    .iter()
                    .find(|(id, _)| id == &input.provider_id)
                    .map(|(_, provider)| Arc::clone(provider))
                else {
                    let error = ProcessModuleRpcError::new(
                        -32602,
                        format!("unknown context provider: {}", input.provider_id),
                    );
                    return Box::pin(async move { Err(error) });
                };
                let provider_input = ContextBuildInput {
                    task: input.task,
                    search: Arc::clone(&self.input.search),
                    memory: Arc::clone(&self.input.memory),
                };
                Box::pin(
                    async move { host_result(provider.provide(&provider_input).await, &method) },
                )
            }
            _ => Box::pin(async move {
                Err(ProcessModuleRpcError::new(
                    -32601,
                    format!("context host method is not implemented: {method}"),
                ))
            }),
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
        let response: ProcessContextChunksResponse = self
            .client
            .invoke(PROCESS_CONTEXT_PROVIDER_METHOD, &request)
            .await?;
        Ok(response.result)
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
