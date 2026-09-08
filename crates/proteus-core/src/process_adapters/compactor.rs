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

pub struct ProcessHistoryCompactor {
    client: Arc<ProcessExportClient>,
}

impl ProcessHistoryCompactor {
    pub fn new(
        config: ProcessExportConfig,
        workspace: &Path,
        workflow_timeout_ms: u64,
    ) -> Result<Self> {
        // Compaction can perform multiple model calls. Its inherited total
        // budget belongs to the enclosing workflow, not to one sampling call.
        // Leave settlement to the outer timeout; an explicit export override
        // can intentionally impose a shorter operation budget. An unbounded
        // workflow requires an explicit finite export timeout.
        let timeout_ms = if workflow_timeout_ms == 0 {
            0
        } else {
            workflow_timeout_ms.saturating_add(1_000)
        };
        Ok(Self {
            client: Arc::new(ProcessExportClient::connect(
                "compactor",
                PROCESS_COMPACTOR_CONTRACT_VERSION,
                config,
                workspace,
                timeout_ms,
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
        crate::contracts::validate_compaction_output(&input, &response.output)?;
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
                let failure = crate::model_standard::ModelFailure::from_error(&error);
                ProcessModuleRpcError::new(
                    -32_100,
                    format!("compactor model callback failed: {error:#}"),
                )
                .with_data(serde_json::to_value(failure).expect("model failure serialization"))
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
