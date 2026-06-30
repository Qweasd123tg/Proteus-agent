use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{
    contracts::ToolSource,
    core::{AppConfig, ModuleCatalogEntrySummary, ModuleEpoch, PluginLoadReport},
    domain::{PermissionMode, ToolSpec},
};

pub struct TopologyBuildInput<'a> {
    pub config: &'a AppConfig,
    pub config_path: Option<&'a Path>,
    pub cwd: &'a Path,
    pub catalog_entries: &'a [ModuleCatalogEntrySummary],
    pub tools: &'a [(ToolSource, ToolSpec)],
    pub plugin_reports: &'a [PluginLoadReport],
    pub module_epoch: ModuleEpoch,
    pub permission_mode: PermissionMode,
    pub extra_warnings: Vec<TopologyWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologySnapshot {
    pub profile: String,
    pub cwd: String,
    pub config_path: Option<String>,
    pub config_files: Vec<String>,
    pub module_epoch: u64,
    pub permission_mode: String,
    pub model: Option<ModelTopology>,
    pub slots: Vec<SlotTopology>,
    pub modules: Vec<ModuleTopology>,
    pub plugins: Vec<PluginTopology>,
    pub tools: Vec<ToolTopology>,
    pub edges: Vec<TopologyEdge>,
    pub warnings: Vec<TopologyWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelTopology {
    pub provider: String,
    pub name: String,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotTopology {
    pub id: String,
    pub title: String,
    pub responsibility: String,
    pub active_module: Option<String>,
    pub required: bool,
    /// Группа slot для рендереров: orchestrator | pipeline | registry |
    /// backend | post_turn | custom. Группировка задаётся здесь, чтобы
    /// клиенты не хардкодили свои списки.
    #[serde(default)]
    pub category: String,
    /// Порядок отображения внутри snapshot: turn pipeline сначала, затем
    /// backends и post-turn. Custom slots получают большой order.
    #[serde(default)]
    pub order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleTopology {
    pub id: String,
    pub slot: String,
    pub active: bool,
    pub source: ModuleSourceTopology,
    pub version: String,
    pub api_version: String,
    pub capabilities: Vec<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModuleSourceTopology {
    Builtin,
    Plugin { name: String, path: String },
    Config,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginTopology {
    pub name: String,
    pub version: String,
    pub path: String,
    pub status: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub tags: Vec<String>,
    pub provides: PluginProvidesTopology,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginProvidesTopology {
    pub modules: Vec<PluginModuleContributionTopology>,
    pub tools: Vec<PluginToolContributionTopology>,
    pub context_providers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginModuleContributionTopology {
    pub slot: String,
    pub id: String,
    pub description: Option<String>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToolContributionTopology {
    pub name: String,
    pub description: String,
    pub safety: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolTopology {
    pub name: String,
    pub description: String,
    pub safety: String,
    pub source: String,
    pub enabled: bool,
    pub registered: bool,
    pub provider_plugin: Option<String>,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyWarning {
    pub severity: String,
    pub message: String,
}

impl TopologyWarning {
    pub fn warn(message: impl Into<String>) -> Self {
        Self {
            severity: "warning".to_owned(),
            message: message.into(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: "error".to_owned(),
            message: message.into(),
        }
    }
}
