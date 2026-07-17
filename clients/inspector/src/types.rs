use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SessionToken(Option<String>);

impl SessionToken {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            Self(None)
        } else {
            Self(Some(value.to_owned()))
        }
    }

    pub(crate) fn missing() -> Self {
        Self(None)
    }

    pub(crate) fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigSummary {
    pub(crate) config_path: Option<String>,
    pub(crate) config_files: Vec<String>,
    pub(crate) cwd: String,
    pub(crate) session_dir: Option<String>,
    pub(crate) profile: String,
    pub(crate) model: ConfigModel,
    pub(crate) model_options: Vec<ConfigModel>,
    pub(crate) reasoning: ConfigReasoning,
    pub(crate) permission_mode: String,
    pub(crate) modules: Vec<ConfigModule>,
    pub(crate) tools_enabled: Vec<String>,
    pub(crate) registered_tools: Vec<ConfigTool>,
    pub(crate) plugins: Vec<ConfigPlugin>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigModel {
    pub(crate) provider: String,
    pub(crate) name: String,
    pub(crate) label: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigReasoning {
    pub(crate) enabled: bool,
    pub(crate) effort: Option<String>,
    pub(crate) effort_options: Vec<String>,
    pub(crate) summary: bool,
    pub(crate) budget_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigModule {
    pub(crate) slot: String,
    pub(crate) id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigTool {
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) safety: String,
    pub(crate) description: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigPlugin {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) status: String,
    pub(crate) description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct ConfigBuilderSnapshot {
    pub(crate) config_path: Option<String>,
    pub(crate) target_path: Option<String>,
    pub(crate) writable: bool,
    pub(crate) active_provider: Option<String>,
    pub(crate) providers: Vec<ConfigBuilderProvider>,
    pub(crate) permission_mode: String,
    pub(crate) permission_modes: Vec<String>,
    pub(crate) active_modules: Vec<ConfigModule>,
    pub(crate) module_config: BTreeMap<String, BTreeMap<String, Value>>,
    pub(crate) tools_enabled: Vec<String>,
    pub(crate) tools: Vec<ConfigBuilderTool>,
    pub(crate) slots: Vec<ConfigBuilderSlot>,
    pub(crate) warnings: Vec<ConfigBuilderWarning>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigBuilderProvider {
    pub(crate) id: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) label: String,
    pub(crate) active: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct ConfigBuilderTool {
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) safety: String,
    pub(crate) description: String,
    pub(crate) enabled: bool,
    pub(crate) registered: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct ConfigBuilderSlot {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) responsibility: String,
    pub(crate) active_module: Option<String>,
    pub(crate) required: bool,
    pub(crate) category: String,
    pub(crate) order: u32,
    pub(crate) modules: Vec<ConfigBuilderModule>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct ConfigBuilderModule {
    pub(crate) id: String,
    pub(crate) slot: String,
    pub(crate) active: bool,
    pub(crate) source: String,
    pub(crate) version: String,
    pub(crate) api_version: String,
    pub(crate) capabilities: Vec<String>,
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigBuilderWarning {
    pub(crate) severity: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct ConfigBuilderSaveRequest {
    pub(crate) modules: BTreeMap<String, String>,
    pub(crate) module_config: BTreeMap<String, BTreeMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools_enabled: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) active_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) permission_mode: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologySnapshot {
    pub(crate) profile: String,
    pub(crate) cwd: String,
    pub(crate) config_path: Option<String>,
    pub(crate) config_files: Vec<String>,
    pub(crate) module_epoch: u64,
    pub(crate) permission_mode: String,
    pub(crate) model: Option<TopologyModel>,
    pub(crate) slots: Vec<TopologySlot>,
    pub(crate) modules: Vec<TopologyModule>,
    pub(crate) plugins: Vec<TopologyPlugin>,
    pub(crate) tools: Vec<TopologyTool>,
    pub(crate) edges: Vec<TopologyEdge>,
    pub(crate) warnings: Vec<TopologyWarning>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyModel {
    pub(crate) provider: String,
    pub(crate) name: String,
    pub(crate) stream: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologySlot {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) responsibility: String,
    pub(crate) active_module: Option<String>,
    pub(crate) required: bool,
    pub(crate) category: String,
    pub(crate) order: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyModule {
    pub(crate) id: String,
    pub(crate) slot: String,
    pub(crate) active: bool,
    pub(crate) source: TopologyModuleSource,
    pub(crate) version: String,
    pub(crate) api_version: String,
    pub(crate) capabilities: Vec<String>,
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyModuleSource {
    pub(crate) kind: String,
    pub(crate) name: Option<String>,
    pub(crate) path: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyPlugin {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) path: String,
    pub(crate) status: String,
    pub(crate) description: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) provides: TopologyPluginProvides,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyPluginProvides {
    pub(crate) modules: Vec<TopologyPluginModuleContribution>,
    pub(crate) tools: Vec<TopologyPluginToolContribution>,
    pub(crate) context_providers: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct TopologyPluginModuleContribution {
    pub(crate) slot: String,
    pub(crate) id: String,
    pub(crate) description: Option<String>,
    pub(crate) capabilities: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyPluginToolContribution {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) safety: String,
    pub(crate) input_schema: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) safety: String,
    pub(crate) source: String,
    pub(crate) enabled: bool,
    pub(crate) registered: bool,
    pub(crate) provider_plugin: Option<String>,
    pub(crate) input_schema: Value,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct TopologyEdge {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) kind: String,
    pub(crate) label: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct TopologyWarning {
    pub(crate) severity: String,
    pub(crate) message: String,
}
