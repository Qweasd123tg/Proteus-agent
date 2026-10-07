mod persistence;
#[cfg(test)]
mod tests;
mod transaction;
pub(super) use persistence::config_builder_target_path;
use persistence::{persist_config_builder, validate_module_config_toml};
pub(super) use transaction::path_lock;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::{
    core::{
        AppConfig, ModuleCatalogEntrySummary, ModuleSourceTopology, ModuleTopology, ModulesConfig,
        ProviderProfileConfig, TopologySnapshot,
        core_slots::{CoreSlotSelection, core_slot_descriptor_by_id},
    },
    domain::PermissionMode,
};

use super::{
    AppServerHandle,
    config_history::{config_history_dir, record_replaced_state},
    prepare_assembly,
};

pub use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderModule, ConfigBuilderModuleSelection, ConfigBuilderProvider, ConfigBuilderSlot,
    ConfigBuilderSnapshot, ConfigBuilderState, ConfigBuilderTool, ConfigBuilderWarning,
};

impl AppServerHandle {
    pub async fn config_builder_snapshot(&self) -> ConfigBuilderSnapshot {
        let topology = self.topology_snapshot().await;
        let config = self.config.read().await.clone();
        let mut snapshot = config_builder_snapshot_from_topology(&topology, &config);
        if let Some(message) = self.profile_error.lock().await.clone() {
            snapshot.warnings.push(ConfigBuilderWarning {
                severity: "error".into(),
                message,
            });
        }
        let descriptions = self.runtime.config_schemas().await;
        for module in snapshot
            .slots
            .iter_mut()
            .flat_map(|slot| &mut slot.modules)
            .chain(snapshot.hook_modules.iter_mut())
            .chain(snapshot.model_modules.iter_mut())
        {
            module.config_schema = descriptions
                .schemas
                .get(&(module.slot.clone(), module.id.clone()))
                .cloned();
        }
        snapshot
            .warnings
            .extend(
                descriptions
                    .errors
                    .into_iter()
                    .map(|message| ConfigBuilderWarning {
                        severity: "warning".into(),
                        message,
                    }),
            );
        snapshot
    }

    pub async fn set_config_builder(
        &self,
        modules: BTreeMap<String, String>,
        hooks: Option<Vec<String>>,
        module_config: BTreeMap<String, BTreeMap<String, Value>>,
        tools_enabled: Option<Vec<String>>,
        active_provider: Option<String>,
        permission_mode: Option<PermissionMode>,
    ) -> Result<ConfigBuilderSnapshot> {
        self.set_profile_config(
            proteus_contracts::app_protocol::http::SetConfigBuilderRequest {
                modules,
                hooks,
                module_config,
                tools_enabled,
                active_provider,
                permission_mode,
                addon_settings: None,
            },
        )
        .await
    }

    pub async fn set_profile_config(
        &self,
        request: proteus_contracts::app_protocol::http::SetConfigBuilderRequest,
    ) -> Result<ConfigBuilderSnapshot> {
        let this = self.clone();
        tokio::spawn(async move { this.set_config_builder_owned(request).await })
            .await
            .context("config save task failed")?
    }

    async fn set_config_builder_owned(
        &self,
        request: proteus_contracts::app_protocol::http::SetConfigBuilderRequest,
    ) -> Result<ConfigBuilderSnapshot> {
        let proteus_contracts::app_protocol::http::SetConfigBuilderRequest {
            modules,
            hooks,
            module_config,
            tools_enabled,
            active_provider,
            permission_mode,
            addon_settings,
        } = request;
        let config_path = self
            .config_path
            .as_deref()
            .ok_or_else(|| anyhow!("config path is not available; cannot persist config"))?;
        let path_lock = transaction::path_lock(config_path)?;
        let _path_guard = path_lock.lock_owned().await;
        let current_plan = self.runtime.assembly_plan().await;
        validate_config_builder_modules(&modules, current_plan.catalog_entries())?;

        let mut next_config = if tokio::fs::try_exists(config_path).await? {
            AppConfig::load(Some(config_path)).await?
        } else {
            self.config.read().await.clone()
        };
        let replaced_state = config_builder_state(&next_config);
        if let Some(update) = addon_settings {
            if update
                .addons
                .disabled_skills
                .iter()
                .any(|id| id.trim().is_empty())
            {
                anyhow::bail!("disabled skill id must not be empty");
            }
            let resolved = crate::core::agent_plugins::resolve(&update.addons, &self.cwd);
            if let Some(plugin) = resolved
                .plugins
                .iter()
                .find(|plugin| plugin.error.is_some())
            {
                anyhow::bail!(
                    "invalid Agent Plugin {}: {}",
                    plugin.path.display(),
                    plugin.error.as_ref().unwrap()
                );
            }
            next_config.addons = update.addons;
            next_config.tools.mcp_servers = update.mcp_servers;
        }
        if let Some(active_provider) = &active_provider {
            validate_config_builder_provider(active_provider, &next_config)?;
        }
        if let Some(hooks) = hooks {
            next_config.modules.hooks = hooks;
        }
        next_config.modules.validate_hooks()?;
        for (slot, module_id) in modules {
            set_module_slot(&mut next_config.modules, &slot, module_id)?;
        }
        for (slot, values) in module_config {
            next_config.module_config.insert(slot, values);
        }
        if let Some(tools_enabled) = tools_enabled {
            next_config.tools.enabled = tools_enabled;
        }
        if let Some(active_provider) = active_provider {
            next_config.active_provider = Some(active_provider);
        }
        if let Some(mode) = permission_mode {
            next_config.permissions.mode = mode;
        }
        validate_module_config_toml(&next_config.module_config)?;

        let assembly =
            prepare_assembly(&next_config, &self.cwd, self.config_path.as_deref()).await?;
        let (Some(config_path), Some(target_path)) = (
            self.config_path.as_deref(),
            config_builder_target_path(self.config_path.as_deref()),
        ) else {
            anyhow::bail!("config path is not available; cannot persist config");
        };
        self.publish_profile(next_config.clone(), assembly, permission_mode, || async {
            if replaced_state != config_builder_state(&next_config) {
                record_replaced_state(&config_history_dir(config_path), replaced_state)
                    .await
                    .context("failed to record the replaced profile state")?;
            }
            persist_config_builder(&target_path, &next_config).await
        })
        .await?;

        Ok(self.config_builder_snapshot().await)
    }
}

/// Builder-managed fields of a profile, shared by the snapshot and history.
pub(super) fn config_builder_state(config: &AppConfig) -> ConfigBuilderState {
    ConfigBuilderState {
        addon_settings: proteus_contracts::app_protocol::addons::AppAddonsUpdate {
            addons: config.addons.clone(),
            mcp_servers: config.tools.mcp_servers.clone(),
        },
        active_provider: config.active_provider.clone(),
        permission_mode: permission_mode_str(config.permissions.mode),
        active_modules: config
            .modules
            .iter()
            .map(|(kind, id)| ConfigBuilderModuleSelection {
                slot: kind.as_str().to_owned(),
                id: id.to_owned(),
            })
            .collect(),
        hooks: config.modules.hooks.clone(),
        module_config: config.module_config.clone(),
        tools_enabled: config.tools.enabled.clone(),
    }
}

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

    let state = config_builder_state(config);
    ConfigBuilderSnapshot {
        addon_settings: state.addon_settings,
        config_path: topology.config_path.clone(),
        writable: target_path.is_some(),
        target_path: target_path.map(|path| path.display().to_string()),
        active_provider: state.active_provider,
        providers: config_builder_providers(config),
        model_modules: modules
            .iter()
            .filter(|module| module.slot == "model")
            .map(config_builder_module)
            .collect(),
        permission_mode: state.permission_mode,
        permission_modes: PERMISSION_MODES
            .iter()
            .map(|&mode| mode.to_owned())
            .collect(),
        active_modules: state.active_modules,
        hooks: state.hooks,
        hook_modules: modules
            .iter()
            .filter(|module| module.slot == "hook")
            .map(config_builder_module)
            .collect(),
        module_config: state.module_config,
        tools_enabled: state.tools_enabled,
        tools: topology
            .tools
            .iter()
            .map(|tool| ConfigBuilderTool {
                name: tool.name.clone(),
                source: tool.source.clone(),
                safety: tool.safety.clone(),
                description: tool.description.clone(),
                enabled: tool.enabled,
                runtime_managed: tool.runtime_managed,
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
        config_schema: None,
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
        ModuleSourceTopology::Process => "process".to_owned(),
        ModuleSourceTopology::Config => "config".to_owned(),
        ModuleSourceTopology::Unknown => "unknown".to_owned(),
    }
}

fn is_config_builder_module_slot(slot: &str) -> bool {
    core_slot_descriptor_by_id(slot)
        .is_some_and(|descriptor| descriptor.selection == CoreSlotSelection::ModulesConfig)
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

fn config_builder_providers(config: &AppConfig) -> Vec<ConfigBuilderProvider> {
    config
        .providers
        .iter()
        .map(|(id, profile)| ConfigBuilderProvider {
            id: id.clone(),
            provider: profile.provider.clone(),
            model: profile.model.clone(),
            label: format!("{}/{}", profile.provider, profile.model),
            active: config.active_provider.as_ref() == Some(id),
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
    if !modules.set_by_slot_id(slot, module_id) {
        anyhow::bail!("unsupported config builder slot: {slot}");
    }
    Ok(())
}

#[cfg(test)]
mod hook_tests {
    use super::*;

    #[tokio::test]
    async fn builder_save_preserves_hook_order_when_editing_other_selections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = crate::test_model::config();
        config.modules.hooks = vec!["second".into(), "first".into()];
        config.modules.search = Some("rg".into());
        persist_config_builder(&path, &config).await.unwrap();
        config.modules.search = Some("other-search".into());
        persist_config_builder(&path, &config).await.unwrap();
        let saved: toml::Value =
            toml::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(
            saved["modules"]["hooks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
        assert_eq!(saved["modules"]["search"].as_str(), Some("other-search"));
        config.modules.hooks.clear();
        persist_config_builder(&path, &config).await.unwrap();
        let saved: toml::Value =
            toml::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert!(saved["modules"]["hooks"].as_array().unwrap().is_empty());
    }
}
