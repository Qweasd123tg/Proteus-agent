use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigSummary {
    pub display_text: String,
    pub module_epoch: u64,
    pub model_catalog_error: Option<String>,
    pub activity: Option<super::AppSessionActivity>,
    pub config_path: Option<String>,
    pub config_files: Vec<String>,
    pub cwd: String,
    pub session_dir: Option<String>,
    pub profile: String,
    pub model: ConfigModel,
    pub model_options: Vec<ModelOption>,
    pub reasoning: ConfigReasoning,
    pub permission_mode: String,
    pub modules: Vec<ConfigModule>,
    pub tools_enabled: Vec<String>,
    pub registered_tools: Vec<ConfigTool>,
    pub components: Vec<ConfigComponent>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigModel {
    pub provider: String,
    pub name: String,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigReasoning {
    pub enabled: bool,
    pub effort: Option<String>,
    pub effort_options: Vec<String>,
    pub summary: bool,
    pub budget_tokens: Option<u32>,
}

pub use super::config_builder::ConfigBuilderModuleSelection as ConfigModule;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigTool {
    pub supports_parallel_tool_calls: bool,
    pub name: String,
    pub source: String,
    pub safety: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigComponent {
    pub id: String,
    pub exports: Vec<ConfigComponentExport>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigComponentExport {
    pub slot: String,
    pub module_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelOption {
    pub provider: String,
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    pub hidden: bool,
    pub reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
}
