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
    #[serde(default)]
    pub(crate) config_path: Option<String>,
    #[serde(default)]
    pub(crate) config_files: Vec<String>,
    #[serde(default)]
    pub(crate) cwd: String,
    #[serde(default)]
    pub(crate) session_dir: Option<String>,
    #[serde(default)]
    pub(crate) profile: String,
    #[serde(default)]
    pub(crate) model: ConfigModel,
    #[serde(default)]
    pub(crate) model_options: Vec<ConfigModel>,
    #[serde(default)]
    pub(crate) reasoning: ConfigReasoning,
    #[serde(default)]
    pub(crate) permission_mode: String,
    #[serde(default)]
    pub(crate) modules: Vec<ConfigModule>,
    #[serde(default)]
    pub(crate) tools_enabled: Vec<String>,
    #[serde(default)]
    pub(crate) registered_tools: Vec<ConfigTool>,
    #[serde(default)]
    pub(crate) plugins: Vec<ConfigPlugin>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigModel {
    #[serde(default)]
    pub(crate) provider: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) label: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigReasoning {
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) effort: Option<String>,
    #[serde(default)]
    pub(crate) effort_options: Vec<String>,
    #[serde(default)]
    pub(crate) summary: bool,
    #[serde(default)]
    pub(crate) budget_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigModule {
    #[serde(default)]
    pub(crate) slot: String,
    #[serde(default)]
    pub(crate) id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigTool {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) source: String,
    #[serde(default)]
    pub(crate) safety: String,
    #[serde(default)]
    pub(crate) description: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigPlugin {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct ConfigBuilderSnapshot {
    #[serde(default)]
    pub(crate) config_path: Option<String>,
    #[serde(default)]
    pub(crate) target_path: Option<String>,
    #[serde(default)]
    pub(crate) writable: bool,
    #[serde(default)]
    pub(crate) active_modules: Vec<ConfigModule>,
    #[serde(default)]
    pub(crate) module_config: BTreeMap<String, BTreeMap<String, Value>>,
    #[serde(default)]
    pub(crate) tools_enabled: Vec<String>,
    #[serde(default)]
    pub(crate) tools: Vec<ConfigBuilderTool>,
    #[serde(default)]
    pub(crate) slots: Vec<ConfigBuilderSlot>,
    #[serde(default)]
    pub(crate) warnings: Vec<ConfigBuilderWarning>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct ConfigBuilderTool {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) source: String,
    #[serde(default)]
    pub(crate) safety: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) registered: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct ConfigBuilderSlot {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) responsibility: String,
    #[serde(default)]
    pub(crate) active_module: Option<String>,
    #[serde(default)]
    pub(crate) required: bool,
    #[serde(default)]
    pub(crate) category: String,
    #[serde(default)]
    pub(crate) order: u32,
    #[serde(default)]
    pub(crate) modules: Vec<ConfigBuilderModule>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct ConfigBuilderModule {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) slot: String,
    #[serde(default)]
    pub(crate) active: bool,
    #[serde(default)]
    pub(crate) source: String,
    #[serde(default)]
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) api_version: String,
    #[serde(default)]
    pub(crate) capabilities: Vec<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct ConfigBuilderWarning {
    #[serde(default)]
    pub(crate) severity: String,
    #[serde(default)]
    pub(crate) message: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct ConfigBuilderSaveRequest {
    pub(crate) modules: BTreeMap<String, String>,
    pub(crate) module_config: BTreeMap<String, BTreeMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools_enabled: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologySnapshot {
    #[serde(default)]
    pub(crate) profile: String,
    #[serde(default)]
    pub(crate) cwd: String,
    #[serde(default)]
    pub(crate) config_path: Option<String>,
    #[serde(default)]
    pub(crate) config_files: Vec<String>,
    #[serde(default)]
    pub(crate) module_epoch: u64,
    #[serde(default)]
    pub(crate) permission_mode: String,
    #[serde(default)]
    pub(crate) model: Option<TopologyModel>,
    #[serde(default)]
    pub(crate) slots: Vec<TopologySlot>,
    #[serde(default)]
    pub(crate) modules: Vec<TopologyModule>,
    #[serde(default)]
    pub(crate) plugins: Vec<TopologyPlugin>,
    #[serde(default)]
    pub(crate) tools: Vec<TopologyTool>,
    #[serde(default)]
    pub(crate) edges: Vec<TopologyEdge>,
    #[serde(default)]
    pub(crate) warnings: Vec<TopologyWarning>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyModel {
    #[serde(default)]
    pub(crate) provider: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) stream: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologySlot {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) responsibility: String,
    #[serde(default)]
    pub(crate) active_module: Option<String>,
    #[serde(default)]
    pub(crate) required: bool,
    #[serde(default)]
    pub(crate) category: String,
    #[serde(default)]
    pub(crate) order: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyModule {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) slot: String,
    #[serde(default)]
    pub(crate) active: bool,
    #[serde(default)]
    pub(crate) source: TopologyModuleSource,
    #[serde(default)]
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) api_version: String,
    #[serde(default)]
    pub(crate) capabilities: Vec<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyModuleSource {
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) path: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyPlugin {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) author: Option<String>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) provides: TopologyPluginProvides,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyPluginProvides {
    #[serde(default)]
    pub(crate) modules: Vec<TopologyPluginModuleContribution>,
    #[serde(default)]
    pub(crate) tools: Vec<TopologyPluginToolContribution>,
    #[serde(default)]
    pub(crate) context_providers: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct TopologyPluginModuleContribution {
    #[serde(default)]
    pub(crate) slot: String,
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) capabilities: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyPluginToolContribution {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) safety: String,
    #[serde(default)]
    pub(crate) input_schema: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct TopologyTool {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) safety: String,
    #[serde(default)]
    pub(crate) source: String,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) registered: bool,
    #[serde(default)]
    pub(crate) provider_plugin: Option<String>,
    #[serde(default)]
    pub(crate) input_schema: Value,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct TopologyEdge {
    #[serde(default)]
    pub(crate) from: String,
    #[serde(default)]
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) label: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub(crate) struct TopologyWarning {
    #[serde(default)]
    pub(crate) severity: String,
    #[serde(default)]
    pub(crate) message: String,
}
