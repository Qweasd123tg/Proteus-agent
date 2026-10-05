use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderSnapshot {
    pub config_path: Option<String>,
    pub target_path: Option<String>,
    pub writable: bool,
    pub active_provider: Option<String>,
    pub providers: Vec<ConfigBuilderProvider>,
    pub model_modules: Vec<ConfigBuilderModule>,
    /// Persisted `[permissions] mode` (snake_case) — то, что редактирует
    /// builder. Runtime mode может отличаться после `POST /mode`.
    pub permission_mode: String,
    pub permission_modes: Vec<String>,
    pub active_modules: Vec<ConfigBuilderModuleSelection>,
    pub hooks: Vec<String>,
    pub hook_modules: Vec<ConfigBuilderModule>,
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
    pub config_schema: Option<crate::domain::ModuleConfigSchema>,
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
    /// `tools_enabled` cannot toggle this tool; preserve any existing entry on save.
    pub runtime_managed: bool,
    pub registered: bool,
}

/// Builder-managed part of a saved profile: the fields `POST /config/builder`
/// writes, in the shape of [`ConfigBuilderSnapshot`].
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderState {
    pub active_provider: Option<String>,
    pub permission_mode: String,
    pub active_modules: Vec<ConfigBuilderModuleSelection>,
    pub hooks: Vec<String>,
    pub module_config: BTreeMap<String, BTreeMap<String, Value>>,
    pub tools_enabled: Vec<String>,
}

/// A builder-managed state that a save replaced.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigRevision {
    pub id: String,
    /// Unix milliseconds of the save that replaced this state.
    pub replaced_at_ms: u64,
    pub state: ConfigBuilderState,
}

/// `GET /config/history`: replaced states of the profile, newest first.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigHistory {
    pub revisions: Vec<ConfigRevision>,
}
