use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleManifest {
    pub id: String,
    pub kind: ModuleKind,
    pub version: String,
    pub api_version: String,
    pub capabilities: Vec<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModuleKind {
    Model,
    Search,
    Memory,
    MemoryPolicy,
    Context,
    Tool,
    Policy,
    Patch,
    Workflow,
    Renderer,
}

impl ModuleManifest {
    pub fn builtin(id: &str, kind: ModuleKind, capabilities: &[&str]) -> Self {
        Self {
            id: id.to_owned(),
            kind,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            api_version: "v0".to_owned(),
            capabilities: capabilities
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
            description: None,
        }
    }
}
