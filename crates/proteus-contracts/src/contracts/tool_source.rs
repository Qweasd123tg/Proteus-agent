/// Host-assigned identity of the process export that supplied a tool.
/// This is provenance, not an additional capability or policy grant.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessToolOwner {
    pub component_id: String,
    pub module_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolSource {
    Builtin { provider: String },
    ProviderHosted { provider: String },
    Config { origin: String },
    Mcp { server: String },
    Process { owner: ProcessToolOwner },
    Dynamic { origin: String },
}

impl ToolSource {
    pub fn builtin(provider: impl Into<String>) -> Self {
        Self::Builtin {
            provider: provider.into(),
        }
    }

    pub fn process_owner(&self) -> Option<&ProcessToolOwner> {
        match self {
            Self::Process { owner } => Some(owner),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Builtin { provider } => format!("builtin:{provider}"),
            Self::ProviderHosted { provider } => format!("provider_hosted:{provider}"),
            Self::Config { origin } => format!("config:{origin}"),
            Self::Mcp { server } => format!("mcp:{server}"),
            Self::Process { owner } => {
                format!("process:{}/{}", owner.component_id, owner.module_id)
            }
            Self::Dynamic { origin } => format!("dynamic:{origin}"),
        }
    }
}
