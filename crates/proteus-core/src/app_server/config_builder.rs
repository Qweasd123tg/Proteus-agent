use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::{
    core::{
        AppConfig, ModuleCatalogEntrySummary, ModuleSourceTopology, ModuleTopology, ModulesConfig,
        TopologySnapshot,
    },
    domain::PermissionMode,
};

use super::module_summary;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigBuilderSnapshot {
    pub config_path: Option<String>,
    pub target_path: Option<String>,
    pub writable: bool,
    /// Выбранный provider с учётом fallback на "default"; `None` — providers
    /// не настроены и модель берётся из секции `[model]`.
    pub active_provider: Option<String>,
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigBuilderProvider {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub label: String,
    pub active: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigBuilderModuleSelection {
    pub slot: String,
    pub id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
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

#[derive(Debug, Clone, serde::Serialize)]
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigBuilderWarning {
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigBuilderTool {
    pub name: String,
    pub source: String,
    pub safety: String,
    pub description: String,
    pub enabled: bool,
    pub registered: bool,
}

const CONFIG_BUILDER_MODULE_SLOTS: [&str; 10] = [
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
];

pub(super) fn config_builder_snapshot_from_topology(
    topology: &TopologySnapshot,
    config: &AppConfig,
) -> ConfigBuilderSnapshot {
    let target_path = config_builder_target_path(topology.config_path.as_deref().map(Path::new));
    let modules = topology.modules.clone();
    let slots = topology
        .slots
        .iter()
        .filter(|slot| is_config_builder_module_slot(&slot.id))
        .map(|slot| ConfigBuilderSlot {
            id: slot.id.clone(),
            title: slot.title.clone(),
            responsibility: slot.responsibility.clone(),
            active_module: slot.active_module.clone(),
            required: slot.required,
            category: slot.category.clone(),
            order: slot.order,
            modules: modules
                .iter()
                .filter(|module| module.slot == slot.id)
                .map(config_builder_module)
                .collect(),
        })
        .collect();

    ConfigBuilderSnapshot {
        config_path: topology.config_path.clone(),
        writable: target_path.is_some(),
        target_path: target_path.map(|path| path.display().to_string()),
        active_provider: resolved_active_provider(config),
        providers: config_builder_providers(config),
        permission_mode: permission_mode_str(config.permissions.mode),
        permission_modes: PERMISSION_MODES
            .iter()
            .map(|&mode| mode.to_owned())
            .collect(),
        active_modules: module_summary(config)
            .into_iter()
            .filter_map(|value| {
                Some(ConfigBuilderModuleSelection {
                    slot: value.get("slot")?.as_str()?.to_owned(),
                    id: value.get("id")?.as_str()?.to_owned(),
                })
            })
            .filter(|selection| is_config_builder_module_slot(&selection.slot))
            .collect(),
        module_config: config.module_config.clone(),
        tools_enabled: config.tools.enabled.clone(),
        tools: topology
            .tools
            .iter()
            .map(|tool| ConfigBuilderTool {
                name: tool.name.clone(),
                source: tool.source.clone(),
                safety: tool.safety.clone(),
                description: tool.description.clone(),
                enabled: tool.enabled,
                registered: tool.registered,
            })
            .collect(),
        warnings: topology
            .warnings
            .iter()
            .map(|warning| ConfigBuilderWarning {
                severity: warning.severity.clone(),
                message: warning.message.clone(),
            })
            .collect(),
        slots,
    }
}

fn config_builder_module(module: &ModuleTopology) -> ConfigBuilderModule {
    ConfigBuilderModule {
        id: module.id.clone(),
        slot: module.slot.clone(),
        active: module.active,
        source: module_source_label(&module.source),
        version: module.version.clone(),
        api_version: module.api_version.clone(),
        capabilities: module.capabilities.clone(),
        description: module.description.clone(),
    }
}

fn module_source_label(source: &ModuleSourceTopology) -> String {
    match source {
        ModuleSourceTopology::Builtin => "builtin".to_owned(),
        // Путь до .so в UI не нужен: плагины лежат в одном plugins dir, а
        // длинные пути делают select нечитаемым. Полный путь остаётся в
        // topology snapshot (`ModuleSourceTopology::Plugin.path`).
        ModuleSourceTopology::Plugin { name, .. } => format!("plugin:{name}"),
        ModuleSourceTopology::Config => "config".to_owned(),
        ModuleSourceTopology::Unknown => "unknown".to_owned(),
    }
}

fn is_config_builder_module_slot(slot: &str) -> bool {
    CONFIG_BUILDER_MODULE_SLOTS.contains(&slot)
}

const PERMISSION_MODES: [&str; 3] = ["plan", "normal", "auto"];

/// Snake_case-имя PermissionMode через serde: остаётся в согласии с wire
/// форматом `POST /mode` и `[permissions] mode` без ручного match.
fn permission_mode_str(mode: PermissionMode) -> String {
    serde_json::to_value(mode)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "normal".to_owned())
}

/// Provider id, который фактически выберет `active_model_config()`:
/// явный `active_provider` или fallback на "default".
fn resolved_active_provider(config: &AppConfig) -> Option<String> {
    if let Some(active) = config
        .active_provider
        .as_ref()
        .filter(|provider| !provider.trim().is_empty())
    {
        return Some(active.clone());
    }
    config
        .providers
        .contains_key("default")
        .then(|| "default".to_owned())
}

fn config_builder_providers(config: &AppConfig) -> Vec<ConfigBuilderProvider> {
    let active = resolved_active_provider(config);
    config
        .providers
        .iter()
        .map(|(id, profile)| ConfigBuilderProvider {
            id: id.clone(),
            provider: profile.provider.clone(),
            model: profile.model.clone(),
            label: format!("{}/{}", profile.provider, profile.model),
            active: active.as_deref() == Some(id.as_str()),
        })
        .collect()
}

pub(super) fn validate_config_builder_provider(
    active_provider: &str,
    config: &AppConfig,
) -> Result<()> {
    if !config.providers.contains_key(active_provider) {
        anyhow::bail!("active_provider is not defined in [providers]: {active_provider}");
    }
    Ok(())
}

pub(super) fn validate_config_builder_modules(
    modules: &BTreeMap<String, String>,
    catalog_entries: &[ModuleCatalogEntrySummary],
) -> Result<()> {
    let known = catalog_entries
        .iter()
        .map(|entry| (entry.slot.as_str(), entry.id.as_str()))
        .collect::<BTreeSet<_>>();
    for (slot, module_id) in modules {
        if !is_config_builder_module_slot(slot) {
            anyhow::bail!("unsupported config builder slot: {slot}");
        }
        if !known.contains(&(slot.as_str(), module_id.as_str())) {
            anyhow::bail!("module is not registered for slot {slot}: {module_id}");
        }
    }
    Ok(())
}

pub(super) fn set_module_slot(
    modules: &mut ModulesConfig,
    slot: &str,
    module_id: String,
) -> Result<()> {
    match slot {
        "workflow" => modules.workflow = module_id,
        "search" => modules.search = module_id,
        "memory" => modules.memory = module_id,
        "memory_policy" => modules.memory_policy = module_id,
        "context" => modules.context = module_id,
        "policy" => modules.policy = module_id,
        "patch" => modules.patch = module_id,
        "compactor" => modules.compactor = module_id,
        "tool_exposure" => modules.tool_exposure = module_id,
        "renderer" => modules.renderer = module_id,
        _ => anyhow::bail!("unsupported config builder slot: {slot}"),
    }
    Ok(())
}

pub(super) fn config_builder_target_path(config_path: Option<&Path>) -> Option<PathBuf> {
    let path = config_path?;
    if path.is_dir() {
        Some(path.join("config.toml"))
    } else {
        Some(path.to_path_buf())
    }
}

#[derive(serde::Serialize)]
struct ModuleConfigToml<'a> {
    module_config: &'a BTreeMap<String, BTreeMap<String, Value>>,
}

pub(super) fn validate_module_config_toml(
    module_config: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<()> {
    module_config_toml_document(module_config).map(|_| ())
}

fn module_config_toml_document(
    module_config: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<toml_edit::DocumentMut> {
    let text = toml::to_string_pretty(&ModuleConfigToml { module_config })
        .context("module_config contains values that cannot be represented as TOML")?;
    text.parse::<toml_edit::DocumentMut>()
        .context("serialized module_config TOML could not be parsed")
}

pub(super) async fn persist_config_builder(path: &Path, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut doc = read_toml_document_or_empty(path).await?;

    // active_provider записываем только когда выбор provider вообще возможен:
    // без [providers] ключ в файле не нужен.
    if let Some(active_provider) = config
        .active_provider
        .as_ref()
        .filter(|provider| !provider.trim().is_empty())
    {
        doc["active_provider"] = toml_edit::value(active_provider.clone());
    }

    if doc
        .get("permissions")
        .is_none_or(|item| !item.is_table_like())
    {
        doc["permissions"] = toml_edit::table();
    }
    doc["permissions"]["mode"] = toml_edit::value(permission_mode_str(config.permissions.mode));

    if doc.get("modules").is_none_or(|item| !item.is_table_like()) {
        doc["modules"] = toml_edit::table();
    }
    doc["modules"]["workflow"] = toml_edit::value(config.modules.workflow.clone());
    doc["modules"]["search"] = toml_edit::value(config.modules.search.clone());
    doc["modules"]["memory"] = toml_edit::value(config.modules.memory.clone());
    doc["modules"]["memory_policy"] = toml_edit::value(config.modules.memory_policy.clone());
    doc["modules"]["context"] = toml_edit::value(config.modules.context.clone());
    doc["modules"]["policy"] = toml_edit::value(config.modules.policy.clone());
    doc["modules"]["patch"] = toml_edit::value(config.modules.patch.clone());
    doc["modules"]["compactor"] = toml_edit::value(config.modules.compactor.clone());
    doc["modules"]["tool_exposure"] = toml_edit::value(config.modules.tool_exposure.clone());
    doc["modules"]["renderer"] = toml_edit::value(config.modules.renderer.clone());

    let module_config_doc = module_config_toml_document(&config.module_config)?;
    if let Some(item) = module_config_doc.as_table().get("module_config") {
        doc["module_config"] = item.clone();
    } else {
        doc["module_config"] = toml_edit::table();
    }

    if doc.get("tools").is_none_or(|item| !item.is_table_like()) {
        doc["tools"] = toml_edit::table();
    }
    doc["tools"]["enabled"] = toml_edit::value(
        config
            .tools
            .enabled
            .iter()
            .cloned()
            .collect::<toml_edit::Array>(),
    );

    tokio::fs::write(path, doc.to_string()).await?;
    Ok(())
}

pub(super) async fn read_toml_document_or_empty(path: &Path) -> Result<toml_edit::DocumentMut> {
    let existing = match tokio::fs::read_to_string(path).await {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read config {}", path.display()));
        }
    };
    existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| anyhow!("failed to parse config TOML at {}: {err}", path.display()))
}
