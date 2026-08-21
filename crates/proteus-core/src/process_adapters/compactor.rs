use std::{path::Path, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;
use proteus_module_protocol::{
    HostRequestDispatcher, ProcessModuleHostRequest, ProcessModuleRpcError,
};
use serde_json::Value;
use tokio::runtime::Handle;

use crate::contracts::{
    COMPACTOR_HOST_COMPLETE_MODEL_METHOD, CompactionHost, CompactionInput, CompactionOutput,
    HistoryCompactor, PROCESS_COMPACTOR_CONTRACT_VERSION, PROCESS_COMPACTOR_METHOD,
    ProcessCompactionResponse, ProcessCompactorCompleteModelInput,
};

use super::{ProcessExportClient, ProcessExportConfig};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct ProcessHistoryCompactor {
    client: Arc<ProcessExportClient>,
}

impl ProcessHistoryCompactor {
    pub fn new(config: ProcessExportConfig, workspace: &Path) -> Result<Self> {
        Ok(Self {
            client: Arc::new(ProcessExportClient::connect(
                "compactor",
                PROCESS_COMPACTOR_CONTRACT_VERSION,
                config,
                workspace,
                DEFAULT_TIMEOUT_MS,
            )?),
        })
    }
}

#[async_trait]
impl HistoryCompactor for ProcessHistoryCompactor {
    async fn compact(
        &self,
        input: CompactionInput,
        host: Arc<dyn CompactionHost>,
    ) -> Result<CompactionOutput> {
        if host.is_cancelled() {
            bail!("turn canceled by client");
        }
        let client = Arc::clone(&self.client);
        let cancellation = Arc::clone(&host);
        let dispatcher: Arc<dyn HostRequestDispatcher> = Arc::new(CompactorDispatcher {
            host,
            handle: Handle::current(),
        });
        let response = tokio::task::spawn_blocking(move || {
            client.invoke_with_dispatcher::<_, ProcessCompactionResponse>(
                PROCESS_COMPACTOR_METHOD,
                &input,
                dispatcher,
                || cancellation.is_cancelled(),
            )
        })
        .await
        .map_err(|error| anyhow::anyhow!("process compactor join error: {error}"))??;
        Ok(response.output)
    }
}

struct CompactorDispatcher {
    host: Arc<dyn CompactionHost>,
    handle: Handle,
}

impl HostRequestDispatcher for CompactorDispatcher {
    fn dispatch(&self, request: ProcessModuleHostRequest) -> Result<Value, ProcessModuleRpcError> {
        if request.method != COMPACTOR_HOST_COMPLETE_MODEL_METHOD {
            return Err(ProcessModuleRpcError::new(
                -32601,
                format!(
                    "compactor host method is not implemented: {}",
                    request.method
                ),
            ));
        }
        let input: ProcessCompactorCompleteModelInput = serde_json::from_value(request.params)
            .map_err(|error| {
                ProcessModuleRpcError::new(
                    -32602,
                    format!("invalid compactor model request: {error}"),
                )
            })?;
        let response = self
            .handle
            .block_on(self.host.complete_model(input.request))
            .map_err(|error| {
                ProcessModuleRpcError::new(
                    -32_100,
                    format!("compactor model callback failed: {error:#}"),
                )
            })?;
        serde_json::to_value(response).map_err(|error| {
            ProcessModuleRpcError::new(
                -32603,
                format!("failed to serialize compactor model response: {error}"),
            )
        })
    }
}
