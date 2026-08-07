use std::{path::Path, sync::Arc};

use crate::contracts::{
    PROCESS_RENDERER_CONTRACT_VERSION, PROCESS_RENDERER_RENDER_METHOD, ProcessRendererInput,
    ProcessRendererResponse, Renderer,
};
use crate::domain::AgentOutput;
use anyhow::Result;

use super::{ProcessAdapterConfig, ProcessModuleClient};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct ProcessRenderer {
    client: Arc<ProcessModuleClient>,
}

impl ProcessRenderer {
    pub fn new(config: ProcessAdapterConfig, workspace: &Path) -> Result<Self> {
        Ok(Self {
            client: Arc::new(ProcessModuleClient::connect(
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
        let response: ProcessRendererResponse = self.client.invoke(
            PROCESS_RENDERER_RENDER_METHOD,
            &ProcessRendererInput {
                output: output.clone(),
            },
        )?;
        Ok(response.result)
    }
}
