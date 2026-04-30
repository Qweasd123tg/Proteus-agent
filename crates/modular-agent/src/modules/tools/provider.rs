use std::sync::Arc;

use anyhow::{Result, bail};

use crate::{
    contracts::{MemoryStore, PatchApplier, ProvidedTool, SearchBackend, Tool, ToolProvider, ToolSource},
    modules::{
        ApplyPatchTool, ListDirTool, ReadFileTool, RememberFactTool, SearchTool, ShellTool,
        WriteFileTool,
    },
};

#[derive(Clone)]
pub struct BuiltinToolProvider {
    enabled: Vec<String>,
    search: Arc<dyn SearchBackend>,
    patch: Arc<dyn PatchApplier>,
    memory: Arc<dyn MemoryStore>,
}

impl BuiltinToolProvider {
    pub fn new(
        enabled: Vec<String>,
        search: Arc<dyn SearchBackend>,
        patch: Arc<dyn PatchApplier>,
        memory: Arc<dyn MemoryStore>,
    ) -> Self {
        Self {
            enabled,
            search,
            patch,
            memory,
        }
    }

    fn source(&self) -> ToolSource {
        ToolSource::builtin(self.name())
    }

    fn boxed_tool(&self, name: &str) -> Result<Arc<dyn Tool>> {
        match name {
            "read_file" => Ok(Arc::new(ReadFileTool)),
            "list_dir" => Ok(Arc::new(ListDirTool)),
            "apply_patch" => Ok(Arc::new(ApplyPatchTool::new(self.patch.clone()))),
            "write_file" => Ok(Arc::new(WriteFileTool)),
            "shell" => Ok(Arc::new(ShellTool)),
            "search" => Ok(Arc::new(SearchTool::new(self.search.clone()))),
            "remember_fact" => Ok(Arc::new(RememberFactTool::new(self.memory.clone()))),
            name => bail!("unsupported tool: {name}"),
        }
    }
}

impl ToolProvider for BuiltinToolProvider {
    fn name(&self) -> &str {
        "builtin"
    }

    fn tools(&self) -> Result<Vec<ProvidedTool>> {
        self.enabled
            .iter()
            .map(|name| {
                Ok(ProvidedTool::new(
                    self.source(),
                    self.boxed_tool(name.as_str())?,
                ))
            })
            .collect()
    }
}
