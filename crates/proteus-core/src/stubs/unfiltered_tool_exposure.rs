use anyhow::Result;
use async_trait::async_trait;

use crate::contracts::{ToolExposure, ToolExposureInput, ToolExposureOutput};

/// Host-owned structural behavior used when no ToolExposure module is
/// selected. It is not registered in ModuleCatalog and has no module id.
#[derive(Debug, Default, Clone)]
pub struct UnfilteredToolExposure;

#[async_trait]
impl ToolExposure for UnfilteredToolExposure {
    async fn select(&self, input: ToolExposureInput) -> Result<ToolExposureOutput> {
        let mut tools = input.candidates;
        if let Some(max_tools) = input.request.max_tools {
            tools.truncate(max_tools);
        }
        Ok(ToolExposureOutput::new(tools))
    }
}
