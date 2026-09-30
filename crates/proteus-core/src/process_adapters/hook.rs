use super::{ProcessExportClient, ProcessExportConfig};
use crate::contracts::{
    CancellationToken, HookHandler, HookInput, HookResponse, PROCESS_HOOK_CONTRACT_VERSION,
    PROCESS_HOOK_INVOKE_METHOD, ProcessHookResponse,
};
use anyhow::Result;
use async_trait::async_trait;
use proteus_module_protocol::v3::NoAsyncHostRequests;
use std::{path::Path, sync::Arc};

pub struct ProcessHookAdapter {
    client: Arc<ProcessExportClient>,
}
impl ProcessHookAdapter {
    pub fn new(config: ProcessExportConfig, workspace: &Path) -> Result<Self> {
        Ok(Self {
            client: Arc::new(ProcessExportClient::connect(
                "hook",
                PROCESS_HOOK_CONTRACT_VERSION,
                config,
                workspace,
                5_000,
            )?),
        })
    }
}
#[async_trait]
impl HookHandler for ProcessHookAdapter {
    async fn invoke(
        &self,
        input: HookInput,
        cancellation: CancellationToken,
    ) -> Result<HookResponse> {
        let response: ProcessHookResponse = self
            .client
            .invoke_with_dispatcher_and_cancel_check(
                PROCESS_HOOK_INVOKE_METHOD,
                &input,
                Arc::new(NoAsyncHostRequests),
                move || cancellation.is_cancelled(),
            )
            .await?;
        Ok(response.result)
    }
}
