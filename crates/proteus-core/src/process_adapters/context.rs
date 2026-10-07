use std::{path::Path, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use proteus_module_protocol::{
    ProcessModuleRpcError,
    v3::{AsyncHostRequestDispatcher, ComponentHostRequest, HostRequestFuture},
};
use serde::Serialize;
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

use super::{
    ProcessExportClient, ProcessExportConfig,
    host_rpc::{callback_error, decode, encode},
};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

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
                let memory_context = self.input.memory_context.clone();
                Box::pin(async move {
                    host_result(memory.recall(input.query, memory_context).await, &method)
                })
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
                    memory_context: self.input.memory_context.clone(),
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
    skills: crate::domain::SkillRuntimeSettings,
}

impl ProcessContextProvider {
    pub fn new(
        config: ProcessExportConfig,
        workspace: &Path,
        skills: crate::domain::SkillRuntimeSettings,
    ) -> Result<Self> {
        let provider_id = config.module_id().to_owned();
        Ok(Self {
            provider_id,
            skills,
            client: Arc::new(ProcessExportClient::connect(
                "context_provider",
                PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION,
                config,
                workspace,
                DEFAULT_TIMEOUT_MS,
            )?),
        })
    }
}

#[async_trait]
impl RepoAwareContextProvider for ProcessContextProvider {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>> {
        let request = crate::contracts::ProcessContextProviderRequest {
            input: ProcessContextProviderInput {
                provider_id: self.provider_id.clone(),
                task: input.task.clone(),
                metadata: Value::Null,
            },
            skills: self.skills.clone(),
        };
        let response: ProcessContextChunksResponse = self
            .client
            .invoke(PROCESS_CONTEXT_PROVIDER_METHOD, &request)
            .await?;
        Ok(response.result)
    }

    async fn skill_catalog(&self, cwd: &Path) -> Result<Option<crate::domain::SkillCatalog>> {
        let mut response: crate::contracts::ProcessSkillCatalogResponse = self
            .client
            .invoke(
                crate::contracts::PROCESS_CONTEXT_PROVIDER_CATALOG_METHOD,
                &crate::contracts::ProcessSkillCatalogInput {
                    cwd: cwd.to_path_buf(),
                    skills: self.skills.clone(),
                },
            )
            .await?;
        if let Some(catalog) = &mut response.result {
            catalog.validate().map_err(anyhow::Error::msg)?;
            for skill in &mut catalog.skills {
                skill.enabled &= !self.skills.disabled.contains(&skill.id);
            }
        }
        Ok(response.result)
    }
}

fn host_result<T: Serialize>(
    result: Result<T>,
    method: &str,
) -> Result<Value, ProcessModuleRpcError> {
    encode(
        result.map_err(|error| callback_error(method, &error))?,
        method,
    )
}
