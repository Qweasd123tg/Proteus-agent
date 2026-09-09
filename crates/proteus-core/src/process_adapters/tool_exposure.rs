use std::{path::Path, sync::Arc};

use crate::contracts::{
    PROCESS_TOOL_EXPOSURE_CONTRACT_VERSION, PROCESS_TOOL_EXPOSURE_SELECT_METHOD,
    ProcessToolExposureInput, ProcessToolExposureResponse, ToolExposure, ToolExposureInput,
    ToolExposureOutput,
};
use anyhow::{Result, ensure};
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
        validate_parallel_permissions(&response.result, &input.input.candidates)?;
        Ok(response.result)
    }
}

fn validate_parallel_permissions(
    output: &ToolExposureOutput,
    candidates: &[crate::domain::ToolSpec],
) -> Result<()> {
    for tool in &output.tools {
        ensure!(
            candidates
                .iter()
                .any(|candidate| candidate.name == tool.name
                    && candidate.supports_parallel_tool_calls == tool.supports_parallel_tool_calls),
            "tool exposure changed the registered parallel permission for {}",
            tool.name
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ToolSafety, ToolSpec};
    use serde_json::json;

    #[test]
    fn selection_preserves_registered_parallel_permission() {
        let tool = ToolSpec::new("probe", "probe", json!({}), ToolSafety::ReadOnly);
        let candidates = [tool.clone()];
        validate_parallel_permissions(&ToolExposureOutput::new(vec![tool.clone()]), &candidates)
            .unwrap();
        let output = ToolExposureOutput::new(vec![tool.with_parallel_tool_calls(true)]);
        assert!(validate_parallel_permissions(&output, &candidates).is_err());
        assert!(validate_parallel_permissions(&output, &[]).is_err());
    }
}
