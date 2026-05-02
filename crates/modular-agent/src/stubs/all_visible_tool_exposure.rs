use anyhow::Result;
use async_trait::async_trait;

use crate::contracts::{ToolExposure, ToolExposureInput, ToolExposureOutput};

#[derive(Debug, Default, Clone)]
pub struct AllVisibleToolExposure;

#[async_trait]
impl ToolExposure for AllVisibleToolExposure {
    async fn select(&self, input: ToolExposureInput) -> Result<ToolExposureOutput> {
        let mut tools = input.candidates;
        if let Some(max_tools) = input.request.max_tools {
            tools.truncate(max_tools);
        }
        Ok(ToolExposureOutput::new(tools))
    }
}
