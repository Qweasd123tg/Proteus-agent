use std::sync::Arc;

use anyhow::Result;

use crate::contracts::{Tool, ToolRegistry, ToolSource};

#[derive(Clone)]
pub struct ProvidedTool {
    pub source: ToolSource,
    pub tool: Arc<dyn Tool>,
}

impl ProvidedTool {
    pub fn new(source: ToolSource, tool: Arc<dyn Tool>) -> Self {
        Self { source, tool }
    }
}

pub trait ToolProvider: Send + Sync {
    fn name(&self) -> &str;
    fn tools(&self) -> Result<Vec<ProvidedTool>>;
}

pub fn register_provider_tools(
    registry: &mut ToolRegistry,
    provider: &dyn ToolProvider,
) -> Result<()> {
    for provided in provider.tools()? {
        registry.register_arc(provided.source, provided.tool)?;
    }
    Ok(())
}
