use std::{collections::BTreeSet, sync::Arc};

use anyhow::{Result, bail};
use proteus_module_protocol::{ProcessExportBinding, current_process_contract_authority};

use crate::{
    contracts::{
        ContextBuilder, HistoryCompactor, MemoryStore, PatchApplier, Renderer, SearchBackend,
        ToolExposure, Workflow,
    },
    core::AppConfig,
    domain::{ModuleKind, ModuleManifest, SlotId, slot},
    process_adapters::{
        ProcessApprovalPolicy, ProcessComponentLauncher, ProcessContextBuilder,
        ProcessHistoryCompactor, ProcessMemoryStore, ProcessPatchApplier, ProcessRenderer,
        ProcessSearchBackend, ProcessToolExposure, ProcessWorkflowAdapter,
    },
};

use super::ModuleCatalog;

impl ModuleCatalog {
    pub(super) fn register_process_components(&mut self, config: &AppConfig) -> Result<()> {
        validate_component_shapes(config)?;

        for (component_id, component) in &config.components {
            let mut bindings = Vec::new();
            for (slot_name, module_id, _) in component.exports() {
                let authority = current_process_contract_authority(slot_name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "component {component_id:?} export {slot_name}/{module_id} targets an unsupported component slot"
                    )
                })?;
                bindings.push(ProcessExportBinding::new(
                    slot_name,
                    module_id,
                    authority.contract_version,
                    config.process_export_config(slot_name, module_id)?,
                )?);
            }

            let launcher =
                ProcessComponentLauncher::new(component_id, component.clone(), bindings)?;
            for (slot_name, module_id, _) in component.exports() {
                let export = launcher.export(slot_name, module_id)?;
                let description = export.description().map(str::to_owned).or_else(|| {
                    Some(format!(
                        "Process component {component_id} export {slot_name}/{module_id}."
                    ))
                });
                self.register_process_export(export, description)?;
            }
        }
        Ok(())
    }

    fn register_process_export(
        &mut self,
        export: crate::process_adapters::ProcessExportConfig,
        description: Option<String>,
    ) -> Result<()> {
        let slot_name = export.slot().to_owned();
        let module_id = export.module_id().to_owned();
        match slot_name.as_str() {
            "tool" => self.process_tools.push(export),
            "context_provider" => self.process_context_providers.push(export),
            "search" => {
                ensure_process_id_is_free(self, slot::SEARCH, &module_id)?;
                self.register_module::<dyn SearchBackend>(
                    slot::SEARCH,
                    &module_id,
                    process_manifest(&module_id, ModuleKind::Search, description),
                    move |ctx| {
                        Ok(Arc::new(ProcessSearchBackend::new(
                            export.clone(),
                            ctx.cwd,
                        )?))
                    },
                );
            }
            "memory" => {
                ensure_process_id_is_free(self, slot::MEMORY, &module_id)?;
                self.register_module::<dyn MemoryStore>(
                    slot::MEMORY,
                    &module_id,
                    process_manifest(&module_id, ModuleKind::Memory, description),
                    move |ctx| Ok(Arc::new(ProcessMemoryStore::new(export.clone(), ctx.cwd)?)),
                );
            }
            "context" => {
                ensure_process_id_is_free(self, slot::CONTEXT, &module_id)?;
                self.register_module::<dyn ContextBuilder>(
                    slot::CONTEXT,
                    &module_id,
                    process_manifest(&module_id, ModuleKind::Context, description),
                    move |ctx| {
                        Ok(Arc::new(ProcessContextBuilder::new(
                            export.clone(),
                            ctx.cwd,
                            ctx.context_providers.to_vec(),
                        )?))
                    },
                );
            }
            "policy" => {
                ensure_process_id_is_free(self, slot::POLICY, &module_id)?;
                self.register_policy(
                    &module_id,
                    process_manifest(&module_id, ModuleKind::Policy, description),
                    move |ctx| {
                        Ok(Arc::new(ProcessApprovalPolicy::new(
                            export.clone(),
                            ctx.cwd,
                        )?))
                    },
                );
            }
            "patch" => {
                ensure_process_id_is_free(self, slot::PATCH, &module_id)?;
                self.register_module::<dyn PatchApplier>(
                    slot::PATCH,
                    &module_id,
                    process_manifest(&module_id, ModuleKind::Patch, description),
                    move |ctx| Ok(Arc::new(ProcessPatchApplier::new(export.clone(), ctx.cwd)?)),
                );
            }
            "compactor" => {
                ensure_process_id_is_free(self, slot::COMPACTOR, &module_id)?;
                self.register_module::<dyn HistoryCompactor>(
                    slot::COMPACTOR,
                    &module_id,
                    process_manifest(&module_id, ModuleKind::Compactor, description),
                    move |ctx| {
                        Ok(Arc::new(ProcessHistoryCompactor::new(
                            export.clone(),
                            ctx.cwd,
                        )?))
                    },
                );
            }
            "tool_exposure" => {
                ensure_process_id_is_free(self, slot::TOOL_EXPOSURE, &module_id)?;
                self.register_module::<dyn ToolExposure>(
                    slot::TOOL_EXPOSURE,
                    &module_id,
                    process_manifest(&module_id, ModuleKind::ToolExposure, description),
                    move |ctx| Ok(Arc::new(ProcessToolExposure::new(export.clone(), ctx.cwd)?)),
                );
            }
            "workflow" => {
                ensure_process_id_is_free(self, slot::WORKFLOW, &module_id)?;
                self.register_module::<dyn Workflow>(
                    slot::WORKFLOW,
                    &module_id,
                    process_manifest(&module_id, ModuleKind::Workflow, description),
                    move |ctx| {
                        Ok(Arc::new(ProcessWorkflowAdapter::new(
                            export.clone(),
                            ctx.cwd,
                            ctx.config.runtime.workflow_timeout_ms,
                        )?))
                    },
                );
            }
            "renderer" => {
                ensure_process_id_is_free(self, slot::RENDERER, &module_id)?;
                self.register_module::<dyn Renderer>(
                    slot::RENDERER,
                    &module_id,
                    process_manifest(&module_id, ModuleKind::Renderer, description),
                    move |ctx| Ok(Arc::new(ProcessRenderer::new(export.clone(), ctx.cwd)?)),
                );
            }
            unsupported => bail!(
                "component export {unsupported}/{module_id} targets an unsupported component slot"
            ),
        }
        Ok(())
    }
}

fn validate_component_shapes(config: &AppConfig) -> Result<()> {
    let mut exports = BTreeSet::new();
    for (component_id, component) in &config.components {
        component.validate_for(component_id, &format!("components.{component_id}"))?;
        for (slot, module_id, _) in component.exports() {
            if !exports.insert((slot.to_owned(), module_id.to_owned())) {
                bail!("duplicate process component export: {slot}/{module_id}");
            }
        }
    }
    Ok(())
}

fn ensure_process_id_is_free(catalog: &ModuleCatalog, slot: SlotId, id: &str) -> Result<()> {
    if catalog.entries.contains_key(&(slot.clone(), id.to_owned())) {
        bail!("process export {slot}/{id} conflicts with an existing catalog entry");
    }
    Ok(())
}

fn process_manifest(id: &str, kind: ModuleKind, description: Option<String>) -> ModuleManifest {
    let contract_version = current_process_contract_authority(kind.as_str())
        .expect("process module kind must have canonical authority")
        .contract_version;
    let mut manifest = ModuleManifest::process(
        id,
        kind,
        contract_version,
        &["process", "component", "stdio", "newline_json"],
    );
    manifest.description = description;
    manifest
}
