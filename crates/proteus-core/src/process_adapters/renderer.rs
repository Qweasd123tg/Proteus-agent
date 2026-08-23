use std::{path::Path, sync::Arc};

use crate::contracts::{
    PROCESS_RENDERER_CONTRACT_VERSION, PROCESS_RENDERER_RENDER_METHOD, ProcessRendererInput,
    ProcessRendererResponse, Renderer,
};
use crate::domain::AgentOutput;
use anyhow::Result;

use super::{ProcessExportClient, ProcessExportConfig};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct ProcessRenderer {
    client: Arc<ProcessExportClient>,
}

impl ProcessRenderer {
    pub fn new(config: ProcessExportConfig, workspace: &Path) -> Result<Self> {
        Ok(Self {
            client: Arc::new(ProcessExportClient::connect(
                "renderer",
                PROCESS_RENDERER_CONTRACT_VERSION,
                config,
                workspace,
                DEFAULT_TIMEOUT_MS,
            )?),
        })
    }
}

impl Renderer for ProcessRenderer {
    fn render(&self, output: &AgentOutput) -> Result<String> {
        let response: ProcessRendererResponse = self.client.invoke_blocking(
            PROCESS_RENDERER_RENDER_METHOD,
            &ProcessRendererInput {
                output: output.clone(),
            },
        )?;
        Ok(response.result)
    }
}
