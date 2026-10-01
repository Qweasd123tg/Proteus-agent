use std::{path::Path, sync::Arc};

use crate::contracts::{
    PROCESS_TOOL_EXPOSURE_CONTRACT_VERSION, PROCESS_TOOL_EXPOSURE_SELECT_METHOD,
    ProcessToolExposureInput, ProcessToolExposureResponse, ToolExposure, ToolExposureInput,
    ToolExposureOutput,
};
use anyhow::Result;
use async_trait::async_trait;

use super::{ProcessExportClient, ProcessExportConfig};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct ProcessToolExposure {
    client: Arc<ProcessExportClient>,
}

impl ProcessToolExposure {
    pub fn new(config: ProcessExportConfig, workspace: &Path) -> Result<Self> {
        Ok(Self {
            client: Arc::new(ProcessExportClient::connect(
                "tool_exposure",
                PROCESS_TOOL_EXPOSURE_CONTRACT_VERSION,
                config,
                workspace,
                DEFAULT_TIMEOUT_MS,
            )?),
        })
    }
}

#[async_trait]
impl ToolExposure for ProcessToolExposure {
    async fn select(&self, input: ToolExposureInput) -> Result<ToolExposureOutput> {
        let input = ProcessToolExposureInput { input };
        let response: ProcessToolExposureResponse = self
            .client
            .invoke(PROCESS_TOOL_EXPOSURE_SELECT_METHOD, &input)
            .await?;
        response.result.validate_against(&input.input.candidates)?;
        Ok(response.result)
    }
}
