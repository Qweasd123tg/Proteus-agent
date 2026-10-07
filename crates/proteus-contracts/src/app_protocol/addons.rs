use crate::domain::{AddonConfig, ConfiguredMcpServerConfig, SkillCatalog};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppAddonsUpdate {
    pub addons: AddonConfig,
    pub mcp_servers: Vec<ConfiguredMcpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppAddonsSnapshot {
    pub reload_error: Option<String>,
    pub writable: bool,
    pub settings: AppAddonsUpdate,
    pub catalogs: Vec<AppSkillCatalog>,
    pub mcp_servers: Vec<AppMcpServerState>,
    pub plugins: Vec<AppAgentPluginState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSkillCatalog {
    pub provider: String,
    pub catalog: Option<SkillCatalog>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppMcpServerState {
    pub name: String,
    pub enabled: bool,
    /// Tools discovered when this immutable assembly was prepared.
    pub tools: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppAgentPluginState {
    pub path: std::path::PathBuf,
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}
