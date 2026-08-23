use std::{path::Path, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;
use proteus_module_protocol::{
    ProcessModuleRpcError,
    v3::{AsyncHostRequestDispatcher, ComponentHostRequest, HostRequestFuture},
};

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
        let cancellation = Arc::clone(&host);
        let dispatcher: Arc<dyn AsyncHostRequestDispatcher> =
            Arc::new(CompactorDispatcher { host });
        let response: ProcessCompactionResponse = self
            .client
            .invoke_with_dispatcher_and_cancel_check(
                PROCESS_COMPACTOR_METHOD,
                &input,
                dispatcher,
                || cancellation.is_cancelled(),
            )
            .await?;
        Ok(response.output)
    }
}

struct CompactorDispatcher {
    host: Arc<dyn CompactionHost>,
}

impl AsyncHostRequestDispatcher for CompactorDispatcher {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        if request.method != COMPACTOR_HOST_COMPLETE_MODEL_METHOD {
            let error = ProcessModuleRpcError::new(
                -32601,
                format!(
                    "compactor host method is not implemented: {}",
                    request.method
                ),
            );
            return Box::pin(async move { Err(error) });
        }
        let input: ProcessCompactorCompleteModelInput = match serde_json::from_value(request.params)
        {
            Ok(input) => input,
            Err(error) => {
                let error = ProcessModuleRpcError::new(
                    -32602,
                    format!("invalid compactor model request: {error}"),
                );
                return Box::pin(async move { Err(error) });
            }
        };
        let host = Arc::clone(&self.host);
        Box::pin(async move {
            let response = host.complete_model(input.request).await.map_err(|error| {
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
        })
    }
}
