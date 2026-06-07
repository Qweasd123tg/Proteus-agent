use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    contracts::ToolSource,
    core::{AppConfig, ModuleCatalogEntrySummary, ModuleEpoch, PluginLoadReport},
    domain::{ModuleManifest, PermissionMode, ToolSafety, ToolSpec},
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

pub fn build_topology_snapshot(input: TopologyBuildInput<'_>) -> TopologySnapshot {
    let config_files = topology_config_files(input.config_path);
    let model_config = input.config.active_model_config();
    let model = model_config.as_ref().ok().map(|model| ModelTopology {
        provider: model.provider.clone(),
        name: model.model.clone(),
        stream: model.stream,
    });
    let active_modules = active_modules(input.config, model.as_ref());
    let plugin_module_sources = plugin_module_sources(input.plugin_reports);
    let plugin_tool_sources = plugin_tool_sources(input.plugin_reports);
    let mut warnings = input.extra_warnings;

    if let Err(error) = &model_config {
        warnings.push(TopologyWarning::error(format!(
            "active model config is invalid: {error:#}"
        )));
    }
    if config_files.len() > 1 {
        warnings.push(TopologyWarning::warn(format!(
            "config path expands to multiple files: {}",
            config_files
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    for report in input.plugin_reports {
        if let Err(error) = &report.result {
            warnings.push(TopologyWarning::error(format!(
                "plugin failed: {}: {}",
                report.path.display(),
                first_line(&error.to_string())
            )));
        }
    }

    let slots = build_slots(input.catalog_entries, &active_modules);
    let modules = build_modules(
        input.catalog_entries,
        &active_modules,
        &plugin_module_sources,
        &mut warnings,
    );
    let plugins = build_plugins(input.plugin_reports);
    let tools = build_tools(
        input.config,
        input.tools,
        &plugin_tool_sources,
        &mut warnings,
    );
    if input.config.modules.tool_exposure == "all_visible"
        && tools.iter().filter(|t| t.registered).count() > 10
    {
        warnings.push(TopologyWarning::warn(
            "tool_exposure=all_visible exposes many registered tools; consider modules.tool_exposure=dynamic",
        ));
    }
    let edges = build_edges(&active_modules, &modules, &plugins, &tools);

    TopologySnapshot {
        profile: input.config.profile.name.clone(),
        cwd: input.cwd.display().to_string(),
        config_path: input.config_path.map(|path| path.display().to_string()),
        config_files: config_files
            .into_iter()
            .map(|path| path.display().to_string())
            .collect(),
        module_epoch: input.module_epoch.as_u64(),
        permission_mode: format!("{:?}", input.permission_mode),
        model,
        slots,
        modules,
        plugins,
        tools,
        edges,
        warnings,
    }
}

pub fn topology_config_files(config_path: Option<&Path>) -> Vec<PathBuf> {
    let Some(path) = config_path else {
        return Vec::new();
    };
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| matches!(extension, "toml" | "json"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn active_modules(config: &AppConfig, model: Option<&ModelTopology>) -> BTreeMap<String, String> {
    let mut modules = BTreeMap::new();
    if let Some(model) = model {
        modules.insert("model".to_owned(), model.provider.clone());
    }
    modules.insert("workflow".to_owned(), config.modules.workflow.clone());
    modules.insert("context".to_owned(), config.modules.context.clone());
    modules.insert(
        "tool_exposure".to_owned(),
        config.modules.tool_exposure.clone(),
    );
    modules.insert("policy".to_owned(), config.modules.policy.clone());
    modules.insert("search".to_owned(), config.modules.search.clone());
    modules.insert("patch".to_owned(), config.modules.patch.clone());
    modules.insert("memory".to_owned(), config.modules.memory.clone());
    modules.insert(
        "memory_policy".to_owned(),
        config.modules.memory_policy.clone(),
    );
    modules.insert("compactor".to_owned(), config.modules.compactor.clone());
    modules.insert("renderer".to_owned(), config.modules.renderer.clone());
    modules
}

fn build_slots(
    catalog_entries: &[ModuleCatalogEntrySummary],
    active_modules: &BTreeMap<String, String>,
) -> Vec<SlotTopology> {
    let mut slot_ids = [
        "model",
        "workflow",
        "context",
        "tool_exposure",
        "policy",
        "search",
        "patch",
        "memory",
        "memory_policy",
        "compactor",
        "renderer",
        "tool",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    slot_ids.extend(catalog_entries.iter().map(|entry| entry.slot.clone()));

    slot_ids
        .into_iter()
        .map(|id| SlotTopology {
            title: slot_title(&id).to_owned(),
            responsibility: slot_responsibility(&id).to_owned(),
            active_module: active_modules.get(&id).cloned(),
            required: slot_required(&id),
            id,
        })
        .collect()
}

fn build_modules(
    catalog_entries: &[ModuleCatalogEntrySummary],
    active_modules: &BTreeMap<String, String>,
    plugin_module_sources: &BTreeMap<(String, String), PluginSource>,
    warnings: &mut Vec<TopologyWarning>,
) -> Vec<ModuleTopology> {
    let mut modules = catalog_entries
        .iter()
        .map(|entry| {
            let active = active_modules
                .get(&entry.slot)
                .is_some_and(|id| id == &entry.id);
            ModuleTopology {
                id: entry.id.clone(),
                slot: entry.slot.clone(),
                active,
                source: module_source(entry, plugin_module_sources),
                version: entry.manifest.version.clone(),
                api_version: entry.manifest.api_version.clone(),
                capabilities: entry.manifest.capabilities.clone(),
                description: entry.manifest.description.clone(),
            }
        })
        .collect::<Vec<_>>();

    let known = catalog_entries
        .iter()
        .map(|entry| (entry.slot.clone(), entry.id.clone()))
        .collect::<BTreeSet<_>>();
    for (slot, id) in active_modules {
        if !known.contains(&(slot.clone(), id.clone())) {
            warnings.push(TopologyWarning::error(format!(
                "active module is not registered: {slot}/{id}"
            )));
            modules.push(ModuleTopology {
                id: id.clone(),
                slot: slot.clone(),
                active: true,
                source: ModuleSourceTopology::Unknown,
                version: String::new(),
                api_version: String::new(),
                capabilities: Vec::new(),
                description: None,
            });
        }
    }

    modules.sort_by(|left, right| {
        left.slot
            .cmp(&right.slot)
            .then_with(|| left.id.cmp(&right.id))
    });
    modules
}

fn module_source(
    entry: &ModuleCatalogEntrySummary,
    plugin_module_sources: &BTreeMap<(String, String), PluginSource>,
) -> ModuleSourceTopology {
    if let Some(source) = plugin_module_sources.get(&(entry.slot.clone(), entry.id.clone())) {
        return ModuleSourceTopology::Plugin {
            name: source.name.clone(),
            path: source.path.clone(),
        };
    }
    if manifest_looks_plugin_owned(&entry.manifest) {
        ModuleSourceTopology::Unknown
    } else {
        ModuleSourceTopology::Builtin
    }
}

fn build_plugins(reports: &[PluginLoadReport]) -> Vec<PluginTopology> {
    reports
        .iter()
        .map(|report| {
            let identity = plugin_identity(report);
            let provides = match &report.result {
                Ok(info) => PluginProvidesTopology {
                    modules: info
                        .contributions
                        .modules
                        .iter()
                        .map(|module| PluginModuleContributionTopology {
                            slot: module.slot.clone(),
                            id: module.id.clone(),
                            description: module.description.clone(),
                            capabilities: module.capabilities.clone(),
                        })
                        .collect(),
                    tools: info
                        .contributions
                        .tools
                        .iter()
                        .map(|tool| PluginToolContributionTopology {
                            name: tool.name.clone(),
                            description: tool.description.clone(),
                            safety: tool.safety.clone(),
                            input_schema: tool.input_schema.clone(),
                        })
                        .collect(),
                    context_providers: info.contributions.context_providers.clone(),
                },
                Err(_) => PluginProvidesTopology::default(),
            };
            PluginTopology {
                name: identity.name,
                version: identity.version,
                path: identity.path,
                status: identity.status,
                description: identity.description,
                author: identity.author,
                tags: identity.tags,
                provides,
            }
        })
        .collect()
}

fn build_tools(
    config: &AppConfig,
    registered_tools: &[(ToolSource, ToolSpec)],
    plugin_tool_sources: &BTreeMap<String, PluginToolSource>,
    warnings: &mut Vec<TopologyWarning>,
) -> Vec<ToolTopology> {
    let enabled_names = config
        .tools
        .enabled
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut registered_names = BTreeSet::new();
    let mut tools = registered_tools
        .iter()
        .map(|(source, spec)| {
            registered_names.insert(spec.name.clone());
            let plugin = plugin_tool_sources.get(&spec.name);
            ToolTopology {
                name: spec.name.clone(),
                description: spec.description.clone(),
                safety: tool_safety_label(&spec.safety).to_owned(),
                source: plugin
                    .map(|plugin| format!("dynamic/plugin:{}", plugin.plugin.name))
                    .unwrap_or_else(|| source.label()),
                enabled: tool_enabled(config, source, &spec.name),
                registered: true,
                provider_plugin: plugin.map(|plugin| plugin.plugin.name.clone()),
                input_schema: spec.input_schema.clone(),
            }
        })
        .collect::<Vec<_>>();

    for (name, plugin_tool) in plugin_tool_sources {
        if registered_names.contains(name) {
            continue;
        }
        if !enabled_names.contains(name) {
            warnings.push(TopologyWarning::warn(format!(
                "plugin {} provides tool {name}, but it is not listed in tools.enabled",
                plugin_tool.plugin.name
            )));
        }
        tools.push(ToolTopology {
            name: name.clone(),
            description: plugin_tool.description.clone(),
            safety: plugin_tool.safety.clone(),
            source: format!("plugin:{}", plugin_tool.plugin.name),
            enabled: enabled_names.contains(name),
            registered: false,
            provider_plugin: Some(plugin_tool.plugin.name.clone()),
            input_schema: plugin_tool.input_schema.clone(),
        });
    }

    for name in enabled_names {
        if !registered_names.contains(&name) && !plugin_tool_sources.contains_key(&name) {
            warnings.push(TopologyWarning::warn(format!(
                "tools.enabled contains {name}, but no registered or loaded plugin tool provides it"
            )));
        }
    }

    tools.sort_by(|left, right| left.name.cmp(&right.name));
    tools
}

fn build_edges(
    active_modules: &BTreeMap<String, String>,
    modules: &[ModuleTopology],
    plugins: &[PluginTopology],
    tools: &[ToolTopology],
) -> Vec<TopologyEdge> {
    let mut edges = Vec::new();
    for (slot, module) in active_modules {
        edges.push(edge(
            "config",
            &format!("slot:{slot}"),
            "selects",
            Some(module),
        ));
    }
    for module in modules {
        let module_node = format!("module:{}:{}", module.slot, module.id);
        if module.active {
            edges.push(edge(
                &format!("slot:{}", module.slot),
                &module_node,
                "active_module",
                Some("active"),
            ));
        } else {
            edges.push(edge(
                &format!("slot:{}", module.slot),
                &module_node,
                "available_module",
                Some("available"),
            ));
        }
    }
    for plugin in plugins {
        let plugin_node = format!("plugin:{}", plugin.name);
        if plugin.status != "loaded" {
            edges.push(edge(
                &plugin_node,
                "warnings",
                "load_error",
                Some("load error"),
            ));
            continue;
        }
        for module in &plugin.provides.modules {
            edges.push(edge(
                &plugin_node,
                &format!("module:{}:{}", module.slot, module.id),
                "provides",
                Some("module"),
            ));
        }
        for tool in &plugin.provides.tools {
            edges.push(edge(
                &plugin_node,
                &format!("tool:{}", tool.name),
                "provides",
                Some("tool"),
            ));
        }
        for provider in &plugin.provides.context_providers {
            let provider_node = format!("context_provider:{provider}");
            edges.push(edge(
                &plugin_node,
                &provider_node,
                "provides",
                Some("context provider"),
            ));
            edges.push(edge(
                &provider_node,
                "slot:context",
                "feeds",
                Some("context provider"),
            ));
        }
    }

    for (from, to, label) in [
        ("slot:workflow", "slot:context", "builds context"),
        ("slot:workflow", "slot:tool_exposure", "selects tools"),
        ("slot:workflow", "slot:model", "model call"),
        ("slot:workflow", "slot:policy", "approval gate"),
        ("slot:workflow", "slot:renderer", "final output"),
        ("slot:tool", "tools", "registry"),
        ("slot:tool_exposure", "tools", "visible tools"),
        ("slot:policy", "tools", "execution policy"),
    ] {
        edges.push(edge(from, to, "runtime", Some(label)));
    }

    for tool in tools.iter().filter(|tool| tool.registered) {
        let tool_node = format!("tool:{}", tool.name);
        edges.push(edge(
            "tools",
            &tool_node,
            "registered_tool",
            Some(if tool.enabled {
                "enabled"
            } else {
                "registered"
            }),
        ));
        if tool.enabled {
            edges.push(edge("config", &tool_node, "enables", Some("enabled")));
        }
        match tool.name.as_str() {
            "apply_patch" => edges.push(edge(&tool_node, "slot:patch", "uses", None)),
            "search" | "grep" | "find_files" => {
                edges.push(edge(&tool_node, "slot:search", "uses", None));
            }
            "remember" | "remember_fact" => {
                edges.push(edge(&tool_node, "slot:memory", "uses", None));
            }
            _ => {}
        }
    }
    for tool in tools.iter().filter(|tool| !tool.registered) {
        let tool_node = format!("tool:{}", tool.name);
        edges.push(edge(
            &tool_node,
            "tools",
            "unregistered_tool",
            Some(if tool.enabled {
                "enabled but not registered"
            } else {
                "provided but disabled"
            }),
        ));
        if tool.enabled {
            edges.push(edge("config", &tool_node, "enables", Some("enabled")));
        }
    }

    edges.sort_by(|left, right| {
        left.from
            .cmp(&right.from)
            .then_with(|| left.to.cmp(&right.to))
            .then_with(|| left.kind.cmp(&right.kind))
    });
    edges.dedup_by(|left, right| {
        left.from == right.from
            && left.to == right.to
            && left.kind == right.kind
            && left.label == right.label
    });
    edges
}

fn edge(from: &str, to: &str, kind: &str, label: Option<&str>) -> TopologyEdge {
    TopologyEdge {
        from: from.to_owned(),
        to: to.to_owned(),
        kind: kind.to_owned(),
        label: label.map(str::to_owned),
    }
}

#[derive(Debug, Clone)]
struct PluginSource {
    name: String,
    path: String,
}

#[derive(Debug, Clone)]
struct PluginToolSource {
    plugin: PluginSource,
    description: String,
    safety: String,
    input_schema: serde_json::Value,
}

fn plugin_module_sources(reports: &[PluginLoadReport]) -> BTreeMap<(String, String), PluginSource> {
    let mut sources = BTreeMap::new();
    for report in reports {
        let Ok(info) = &report.result else {
            continue;
        };
        let identity = plugin_source(report);
        for module in &info.contributions.modules {
            sources.insert(
                (module.slot.clone(), module.id.clone()),
                PluginSource {
                    name: identity.name.clone(),
                    path: identity.path.clone(),
                },
            );
        }
    }
    sources
}

fn plugin_tool_sources(reports: &[PluginLoadReport]) -> BTreeMap<String, PluginToolSource> {
    let mut sources = BTreeMap::new();
    for report in reports {
        let Ok(info) = &report.result else {
            continue;
        };
        let identity = plugin_source(report);
        for tool in &info.contributions.tools {
            sources.insert(
                tool.name.clone(),
                PluginToolSource {
                    plugin: PluginSource {
                        name: identity.name.clone(),
                        path: identity.path.clone(),
                    },
                    description: tool.description.clone(),
                    safety: tool.safety.clone(),
                    input_schema: tool.input_schema.clone(),
                },
            );
        }
    }
    sources
}

fn plugin_source(report: &PluginLoadReport) -> PluginSource {
    let identity = plugin_identity(report);
    PluginSource {
        name: identity.name,
        path: identity.path,
    }
}

struct PluginIdentity {
    name: String,
    version: String,
    path: String,
    status: String,
    description: Option<String>,
    author: Option<String>,
    tags: Vec<String>,
}

fn plugin_identity(report: &PluginLoadReport) -> PluginIdentity {
    let path = report
        .result
        .as_ref()
        .map(|info| info.path.display().to_string())
        .unwrap_or_else(|_| report.path.display().to_string());
    let status = match &report.result {
        Ok(_) => "loaded".to_owned(),
        Err(error) => format!("error: {}", first_line(&error.to_string())),
    };
    match (&report.manifest, &report.result) {
        (Some(manifest), Ok(info)) => PluginIdentity {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            path,
            status,
            description: manifest
                .description
                .clone()
                .or(Some(info.description.clone())),
            author: manifest.author.clone(),
            tags: manifest.tags.clone(),
        },
        (Some(manifest), Err(_)) => PluginIdentity {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            path,
            status,
            description: manifest.description.clone(),
            author: manifest.author.clone(),
            tags: manifest.tags.clone(),
        },
        (None, Ok(info)) => PluginIdentity {
            name: info.name.clone(),
            version: "-".to_owned(),
            path,
            status,
            description: Some(info.description.clone()),
            author: None,
            tags: Vec::new(),
        },
        (None, Err(_)) => PluginIdentity {
            name: report
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| report.path.display().to_string()),
            version: "-".to_owned(),
            path,
            status,
            description: None,
            author: None,
            tags: Vec::new(),
        },
    }
}

fn manifest_looks_plugin_owned(manifest: &ModuleManifest) -> bool {
    manifest
        .capabilities
        .iter()
        .any(|capability| matches!(capability.as_str(), "plugin" | "dylib"))
}

fn tool_enabled(config: &AppConfig, source: &ToolSource, name: &str) -> bool {
    config.tools.enabled.iter().any(|enabled| enabled == name)
        || matches!(source, ToolSource::Config { .. } | ToolSource::Mcp { .. })
}

fn tool_safety_label(safety: &ToolSafety) -> &'static str {
    match safety {
        ToolSafety::ReadOnly => "ReadOnly",
        ToolSafety::WritesFiles => "WritesFiles",
        ToolSafety::RunsCommands => "RunsCommands",
        ToolSafety::Network => "Network",
        ToolSafety::Dangerous => "Dangerous",
        _ => "Unknown",
    }
}

fn slot_title(id: &str) -> &'static str {
    match id {
        "model" => "Model",
        "workflow" => "Workflow",
        "context" => "Context",
        "tool_exposure" => "Tool Exposure",
        "policy" => "Policy",
        "search" => "Search",
        "patch" => "Patch",
        "memory" => "Memory",
        "memory_policy" => "Memory Policy",
        "compactor" => "Compactor",
        "renderer" => "Renderer",
        "tool" => "Tools",
        _ => "Custom Slot",
    }
}

fn slot_responsibility(id: &str) -> &'static str {
    match id {
        "model" => "Adapts canonical model requests to provider APIs.",
        "workflow" => "Controls the agent turn loop: planning, model calls, tool calls, review.",
        "context" => "Builds context chunks before model calls.",
        "tool_exposure" => "Chooses which registered tools are exposed to the model.",
        "policy" => "Evaluates tool execution and approval requirements.",
        "search" => "Provides repository/search backend.",
        "patch" => "Applies structured patches to the workspace.",
        "memory" => "Persists and retrieves memories.",
        "memory_policy" => "Decides what should be remembered after a turn.",
        "compactor" => "Compacts long histories before model requests.",
        "renderer" => "Renders final AgentOutput for clients/CLI.",
        "tool" => "Executable capabilities available through ToolRegistry.",
        _ => "Custom module slot registered in the catalog.",
    }
}

fn slot_required(id: &str) -> bool {
    matches!(
        id,
        "model"
            | "workflow"
            | "context"
            | "tool_exposure"
            | "policy"
            | "patch"
            | "compactor"
            | "renderer"
    )
}

fn first_line(text: &str) -> String {
    let mut lines = text.lines();
    let head = lines.next().unwrap_or("").trim_end().to_owned();
    if lines.next().is_some() {
        format!("{head} ...")
    } else {
        head
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;

    #[test]
    fn build_edges_connects_slots_plugins_modules_tools_and_registry() {
        let active_modules = BTreeMap::from([
            ("workflow".to_owned(), "coding.single_loop".to_owned()),
            ("tool_exposure".to_owned(), "all_visible".to_owned()),
        ]);
        let modules = vec![
            ModuleTopology {
                id: "coding.single_loop".to_owned(),
                slot: "workflow".to_owned(),
                active: true,
                source: ModuleSourceTopology::Plugin {
                    name: "coding-workflow".to_owned(),
                    path: "/plugins/coding-workflow".to_owned(),
                },
                version: "0.1.0".to_owned(),
                api_version: "1".to_owned(),
                capabilities: Vec::new(),
                description: None,
            },
            ModuleTopology {
                id: "none".to_owned(),
                slot: "workflow".to_owned(),
                active: false,
                source: ModuleSourceTopology::Builtin,
                version: "0.1.0".to_owned(),
                api_version: "1".to_owned(),
                capabilities: Vec::new(),
                description: None,
            },
        ];
        let plugins = vec![PluginTopology {
            name: "coding-workflow".to_owned(),
            version: "0.1.0".to_owned(),
            path: "/plugins/coding-workflow".to_owned(),
            status: "loaded".to_owned(),
            description: None,
            author: None,
            tags: Vec::new(),
            provides: PluginProvidesTopology {
                modules: vec![PluginModuleContributionTopology {
                    slot: "workflow".to_owned(),
                    id: "coding.single_loop".to_owned(),
                    description: None,
                    capabilities: Vec::new(),
                }],
                tools: vec![PluginToolContributionTopology {
                    name: "grep".to_owned(),
                    description: "Search files".to_owned(),
                    safety: "ReadOnly".to_owned(),
                    input_schema: json!({ "type": "object" }),
                }],
                context_providers: vec!["repo".to_owned()],
            },
        }];
        let tools = vec![ToolTopology {
            name: "grep".to_owned(),
            description: "Search files".to_owned(),
            safety: "ReadOnly".to_owned(),
            source: "dynamic/plugin:coding-workflow".to_owned(),
            enabled: true,
            registered: true,
            provider_plugin: Some("coding-workflow".to_owned()),
            input_schema: json!({ "type": "object" }),
        }];

        let edges = build_edges(&active_modules, &modules, &plugins, &tools);

        assert!(has_edge(
            &edges,
            "slot:workflow",
            "module:workflow:coding.single_loop",
            "active_module"
        ));
        assert!(has_edge(
            &edges,
            "slot:workflow",
            "module:workflow:none",
            "available_module"
        ));
        assert!(has_edge(
            &edges,
            "plugin:coding-workflow",
            "module:workflow:coding.single_loop",
            "provides"
        ));
        assert!(has_edge(
            &edges,
            "plugin:coding-workflow",
            "tool:grep",
            "provides"
        ));
        assert!(has_edge(&edges, "tools", "tool:grep", "registered_tool"));
        assert!(has_edge(&edges, "config", "tool:grep", "enables"));
        assert!(has_edge(
            &edges,
            "context_provider:repo",
            "slot:context",
            "feeds"
        ));
        assert!(has_edge(&edges, "slot:tool", "tools", "runtime"));
    }

    fn has_edge(edges: &[TopologyEdge], from: &str, to: &str, kind: &str) -> bool {
        edges
            .iter()
            .any(|edge| edge.from == from && edge.to == to && edge.kind == kind)
    }
}
