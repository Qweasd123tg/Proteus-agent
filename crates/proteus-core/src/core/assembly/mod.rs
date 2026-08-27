use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use proteus_module_protocol::current_process_contract_authority;

use crate::{
    core::{
        AppConfig, ModuleCatalog, ModuleCatalogEntrySummary, RuntimeRegistry,
        core_slots::CORE_SLOT_DESCRIPTORS,
    },
    domain::ModuleKind,
};

mod config_files;
mod render;
mod types;

use config_files::assembly_config_files;
pub use render::render_assembly_plan;
pub use types::*;

impl AssemblyPlan {
    pub fn resolve(
        config: AppConfig,
        config_path: Option<&Path>,
        cwd: PathBuf,
        catalog: &ModuleCatalog,
    ) -> Result<Self> {
        let catalog_entries = catalog.entry_summaries();
        let mut checks = Vec::new();
        let model_config = match config.active_model_config() {
            Ok(model) => Some(model),
            Err(error) => {
                checks.push(AssemblyCheck::error(
                    "model_config",
                    format!("active model config is invalid: {error:#}"),
                ));
                None
            }
        };
        let model = model_config.as_ref().map(|model| AssemblyModelPlan {
            profile_id: config.active_provider.clone(),
            provider: model.provider.clone(),
            name: model.model.clone(),
            stream: model.stream,
        });

        let mut active_modules = BTreeMap::new();
        if let Some(model) = &model {
            active_modules.insert(
                ModuleKind::Model.as_str().to_owned(),
                model.provider.clone(),
            );
        }
        active_modules.extend(
            config
                .modules
                .iter()
                .map(|(kind, id)| (kind.as_str().to_owned(), id.to_owned())),
        );

        let known = catalog_entries
            .iter()
            .map(|entry| ((entry.slot.clone(), entry.id.clone()), entry))
            .collect::<BTreeMap<_, _>>();
        let export_components = configured_export_components(&config);
        let slots = CORE_SLOT_DESCRIPTORS
            .iter()
            .map(|descriptor| {
                let slot_id = descriptor.kind.as_str();
                let module_id = active_modules.get(slot_id).cloned();
                let entry = module_id
                    .as_ref()
                    .and_then(|id| known.get(&(slot_id.to_owned(), id.clone())).copied());
                if let Some(module_id) = &module_id
                    && entry.is_none()
                {
                    checks.push(AssemblyCheck::error(
                        "module_not_registered",
                        format!("active module is not registered: {slot_id}/{module_id}"),
                    ));
                }
                let source = module_id.as_ref().map(|_| {
                    entry
                        .map(catalog_module_source)
                        .unwrap_or(AssemblyModuleSource::Unknown)
                });
                let component_id = module_id.as_ref().and_then(|id| {
                    export_components
                        .get(&(slot_id.to_owned(), id.clone()))
                        .cloned()
                });
                AssemblySlotPlan {
                    id: slot_id.to_owned(),
                    title: descriptor.title.to_owned(),
                    responsibility: descriptor.responsibility.to_owned(),
                    required: descriptor.required,
                    category: descriptor.category.to_owned(),
                    order: descriptor.order,
                    module_id,
                    source,
                    component_id,
                }
            })
            .collect::<Vec<_>>();

        check_duplicate_requested_tools(&config, &mut checks);
        let config_files = assembly_config_files(config_path);
        if config_files.len() > 1 {
            checks.push(AssemblyCheck::warning(
                "multiple_config_files",
                format!(
                    "config path expands to multiple files: {}",
                    config_files
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }

        let components = build_components(&config, &active_modules)?;
        let tools = AssemblyToolsPlan {
            requested: config.tools.enabled.clone(),
            configured: config
                .tools
                .configured
                .iter()
                .map(|tool| tool.name.clone())
                .collect(),
            mcp_servers: config
                .tools
                .mcp_servers
                .iter()
                .map(|server| server.name.clone())
                .collect(),
            agent_control_surface: config.agent_control.surface.as_str().to_owned(),
        };

        Ok(Self {
            schema_version: ASSEMBLY_PLAN_SCHEMA_VERSION,
            profile: config.profile.name.clone(),
            config_path: config_path.map(Path::to_path_buf),
            config_files,
            cwd,
            permission_mode: config.permissions.mode,
            model,
            slots,
            components,
            tools,
            checks,
            config,
            catalog_entries,
            active_modules,
        })
    }

    pub fn is_valid(&self) -> bool {
        !self
            .checks
            .iter()
            .any(|check| check.severity == AssemblyCheckSeverity::Error)
    }

    pub fn ensure_valid(&self) -> Result<()> {
        let errors = self
            .checks
            .iter()
            .filter(|check| check.severity == AssemblyCheckSeverity::Error)
            .map(|check| check.message.as_str())
            .collect::<Vec<_>>();
        if errors.is_empty() {
            return Ok(());
        }
        bail!("assembly plan is invalid: {}", errors.join("; "))
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn catalog_entries(&self) -> &[ModuleCatalogEntrySummary] {
        &self.catalog_entries
    }

    pub fn active_modules(&self) -> &BTreeMap<String, String> {
        &self.active_modules
    }

    pub fn module_id(&self, kind: ModuleKind) -> Option<&str> {
        self.active_modules.get(kind.as_str()).map(String::as_str)
    }

    pub fn model_config(&self) -> Result<crate::core::ModelConfig> {
        self.config.active_model_config()
    }
}

/// План и созданные из него runtime-объекты. Bundle не позволяет атомарной
/// reload-операции случайно принять registry от другого config-а.
pub struct PreparedAssembly {
    plan: AssemblyPlan,
    registry: RuntimeRegistry,
}

impl PreparedAssembly {
    pub fn from_config(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&Path>,
    ) -> Result<Self> {
        let catalog = ModuleCatalog::from_config(&config)?;
        Self::from_catalog(config, cwd, config_path, catalog)
    }

    pub fn from_catalog(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&Path>,
        catalog: ModuleCatalog,
    ) -> Result<Self> {
        let plan = AssemblyPlan::resolve(config, config_path, cwd, &catalog)?;
        plan.ensure_valid()?;
        let registry = RuntimeRegistry::from_plan(&plan, catalog)?;
        Ok(Self { plan, registry })
    }

    pub fn plan(&self) -> &AssemblyPlan {
        &self.plan
    }

    pub fn registry(&self) -> &RuntimeRegistry {
        &self.registry
    }

    #[cfg(test)]
    pub(crate) fn registry_mut(&mut self) -> &mut RuntimeRegistry {
        &mut self.registry
    }

    pub fn into_parts(self) -> (AssemblyPlan, RuntimeRegistry) {
        (self.plan, self.registry)
    }
}

fn configured_export_components(config: &AppConfig) -> BTreeMap<(String, String), String> {
    let mut components = BTreeMap::new();
    for (component_id, component) in &config.components {
        for (slot, module_id, _) in component.exports() {
            components.insert(
                (slot.to_owned(), module_id.to_owned()),
                component_id.clone(),
            );
        }
    }
    components
}

fn build_components(
    config: &AppConfig,
    active_modules: &BTreeMap<String, String>,
) -> Result<Vec<AssemblyComponentPlan>> {
    config
        .components
        .iter()
        .map(|(component_id, component)| {
            let exports = component
                .exports()
                .map(|(slot, module_id, _)| {
                    let authority = current_process_contract_authority(slot).ok_or_else(|| {
                        anyhow::anyhow!(
                            "component {component_id:?} export {slot}/{module_id} targets an unsupported component slot"
                        )
                    })?;
                    let use_state = if active_modules
                        .get(slot)
                        .is_some_and(|selected| selected == module_id)
                    {
                        AssemblyExportUse::Selected
                    } else if authority.composition
                        == crate::contracts::ProcessModuleComposition::OrderedMany
                    {
                        AssemblyExportUse::Included
                    } else {
                        AssemblyExportUse::Available
                    };
                    Ok(AssemblyExportPlan {
                        slot: slot.to_owned(),
                        module_id: module_id.to_owned(),
                        contract_version: authority.contract_version.to_owned(),
                        composition: authority.composition,
                        use_state,
                        module_methods: authority
                            .module_methods
                            .iter()
                            .map(|method| (*method).to_owned())
                            .collect(),
                        host_methods: authority
                            .host_methods
                            .iter()
                            .map(|method| (*method).to_owned())
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(AssemblyComponentPlan {
                id: component_id.clone(),
                command: component.command().to_owned(),
                exports,
            })
        })
        .collect()
}

fn check_duplicate_requested_tools(config: &AppConfig, checks: &mut Vec<AssemblyCheck>) {
    let mut seen = BTreeSet::new();
    for name in &config.tools.enabled {
        if !seen.insert(name) {
            checks.push(AssemblyCheck::error(
                "duplicate_tool",
                format!("tools.enabled contains duplicate tool '{name}'"),
            ));
        }
    }
}

pub(crate) fn catalog_module_source(entry: &ModuleCatalogEntrySummary) -> AssemblyModuleSource {
    if entry
        .manifest
        .capabilities
        .iter()
        .any(|capability| capability == "process")
    {
        AssemblyModuleSource::Process
    } else if entry
        .manifest
        .capabilities
        .iter()
        .any(|capability| capability == "config_defined")
    {
        AssemblyModuleSource::Config
    } else {
        AssemblyModuleSource::Builtin
    }
}

#[cfg(test)]
mod tests;
