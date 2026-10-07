//! Host-owned availability and portable package bindings; no skill parser or
//! concrete module implementation lives at this boundary.
use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ToolSafety;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddonConfig {
    #[serde(default)]
    pub disabled_skills: Vec<String>,
    #[serde(default)]
    pub disabled_mcp_servers: Vec<String>,
    #[serde(default)]
    pub plugins: Vec<AgentPluginConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPluginConfig {
    pub path: PathBuf,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRuntimeSettings {
    pub disabled: Vec<String>,
    pub packages: Vec<SkillPackageRoot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillPackageRoot {
    pub id: String,
    pub root: PathBuf,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillCatalog {
    pub skills: Vec<SkillDescriptor>,
    pub warnings: Vec<String>,
}

impl SkillCatalog {
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        for skill in &self.skills {
            if skill.id.trim().is_empty()
                || skill.name.trim().is_empty()
                || skill.description.trim().is_empty()
                || !ids.insert(&skill.id)
            {
                return Err("skill catalog has an empty or duplicate identity/description".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDescriptor {
    /// Invocation name; plugin skills use `plugin-name:skill-name`.
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub source: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessEnvironmentConfig {
    #[serde(default)]
    pub env_allowlist: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredMcpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(flatten)]
    pub environment: ProcessEnvironmentConfig,
    #[serde(default = "mcp_protocol_version")]
    pub protocol_version: String,
    #[serde(default = "mcp_safety")]
    pub safety: ToolSafety,
    #[serde(default)]
    pub supports_parallel_tool_calls: bool,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_response_bytes: Option<usize>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

pub fn mcp_protocol_version() -> String {
    // Canonical default shared with the pinned MCP client and UI consumers.
    "2025-11-25".into()
}

fn mcp_safety() -> ToolSafety {
    ToolSafety::RunsCommands
}

fn enabled() -> bool {
    true
}
