/// A configured process component, called a plugin in the agent settings UI.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderPlugin {
    pub id: String,
    pub command: String,
    pub description: Option<String>,
    pub exports: Vec<ConfigBuilderPluginExport>,
    pub tool_packs: Vec<ConfigBuilderToolPack>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderPluginExport {
    pub slot: String,
    pub id: String,
    pub active: bool,
    pub description: Option<String>,
    pub config_schema: Option<crate::domain::ModuleConfigSchema>,
}

/// The tools supplied by one configured `tool/<module_id>` export.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBuilderToolPack {
    pub id: String,
    pub tools: Vec<String>,
}
