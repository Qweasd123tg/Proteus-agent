use std::collections::{BTreeMap, BTreeSet};

use crate::core::ModuleCatalogEntrySummary;

use super::{
    ModuleSourceTopology, ModuleTopology, TopologyWarning,
    plugins::{PluginSource, manifest_looks_plugin_owned},
};

pub(super) fn build_modules(
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
    } else if entry
        .manifest
        .capabilities
        .iter()
        .any(|capability| capability == "config_defined")
    {
        ModuleSourceTopology::Config
    } else {
        ModuleSourceTopology::Builtin
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ModuleKind, ModuleManifest};

    #[test]
    fn config_defined_process_module_is_reported_as_config_source() {
        let entry = ModuleCatalogEntrySummary {
            slot: "search".to_owned(),
            id: "process".to_owned(),
            manifest: ModuleManifest::builtin(
                "process",
                ModuleKind::Search,
                &["config_defined", "process"],
            ),
        };

        assert_eq!(
            module_source(&entry, &BTreeMap::new()),
            ModuleSourceTopology::Config
        );
    }
}
