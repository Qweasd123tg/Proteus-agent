use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderSnapshot {
    pub config_path: Option<String>,
    pub target_path: Option<String>,
    pub writable: bool,
    pub active_provider: String,
    pub providers: Vec<ConfigBuilderProvider>,
    /// Persisted `[permissions] mode` (snake_case) — то, что редактирует
    /// builder. Runtime mode может отличаться после `POST /mode`.
    pub permission_mode: String,
    pub permission_modes: Vec<String>,
    pub active_modules: Vec<ConfigBuilderModuleSelection>,
    pub module_config: BTreeMap<String, BTreeMap<String, Value>>,
    pub tools_enabled: Vec<String>,
    pub tools: Vec<ConfigBuilderTool>,
    pub slots: Vec<ConfigBuilderSlot>,
    pub warnings: Vec<ConfigBuilderWarning>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderProvider {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub label: String,
    pub active: bool,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderModuleSelection {
    pub slot: String,
    pub id: String,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderSlot {
    pub id: String,
    pub title: String,
    pub responsibility: String,
    pub active_module: Option<String>,
    pub required: bool,
    pub category: String,
    pub order: u32,
    pub modules: Vec<ConfigBuilderModule>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderModule {
    pub id: String,
    pub slot: String,
    pub active: bool,
    pub source: String,
    pub version: String,
    pub api_version: String,
    pub capabilities: Vec<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderWarning {
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderTool {
    pub name: String,
    pub source: String,
    pub safety: String,
    pub description: String,
    pub enabled: bool,
    pub registered: bool,
}
