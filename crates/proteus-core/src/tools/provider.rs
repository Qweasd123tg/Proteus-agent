use std::sync::Arc;

use anyhow::{Result, bail};

use crate::{
    contracts::{MemoryStore, ProvidedTool, SearchBackend, Tool, ToolProvider, ToolSource},
    tools::{RememberFactTool, RequestUserInputTool, SearchTool},
};

#[derive(Clone)]
pub struct BuiltinToolProvider {
    enabled: Vec<String>,
    search: Arc<dyn SearchBackend>,
    memory: Arc<dyn MemoryStore>,
}

impl BuiltinToolProvider {
    pub fn new(
        enabled: Vec<String>,
        search: Arc<dyn SearchBackend>,
        memory: Arc<dyn MemoryStore>,
    ) -> Self {
        Self {
            enabled,
            search,
            memory,
        }
    }

    fn source(&self) -> ToolSource {
        ToolSource::builtin(self.name())
    }

    fn boxed_tool(&self, name: &str) -> Result<Arc<dyn Tool>> {
        match name {
            "search" => Ok(Arc::new(SearchTool::new(self.search.clone()))),
            "remember_fact" => Ok(Arc::new(RememberFactTool::new(self.memory.clone()))),
            "request_user_input" => Ok(Arc::new(RequestUserInputTool::new("request_user_input"))),
            "AskUserQuestion" => Ok(Arc::new(RequestUserInputTool::new("AskUserQuestion"))),
            name => bail!(
                "unsupported tool: '{name}'. File I/O (read_file/write_file/list_dir/grep) \
                 is provided by the `reference.tools` process module. Configure that worker \
                 or remove the tool from tools.enabled."
            ),
        }
    }
}

pub fn is_builtin_tool_name(name: &str) -> bool {
    matches!(
        name,
        "search" | "remember_fact" | "request_user_input" | "AskUserQuestion"
    )
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
