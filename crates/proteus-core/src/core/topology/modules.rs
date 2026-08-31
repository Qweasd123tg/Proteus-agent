use std::collections::{BTreeMap, BTreeSet};

use crate::core::{AssemblyModuleSource, ModuleCatalogEntrySummary, catalog_module_source};

use super::{ModuleSourceTopology, ModuleTopology};

pub(super) fn build_modules(
    catalog_entries: &[ModuleCatalogEntrySummary],
    active_modules: &BTreeMap<String, String>,
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
                source: module_source(entry),
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

fn module_source(entry: &ModuleCatalogEntrySummary) -> ModuleSourceTopology {
    match catalog_module_source(entry) {
        AssemblyModuleSource::Builtin => ModuleSourceTopology::Builtin,
        AssemblyModuleSource::Process => ModuleSourceTopology::Process,
        AssemblyModuleSource::Config => ModuleSourceTopology::Config,
        AssemblyModuleSource::Unknown => ModuleSourceTopology::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ModuleKind, ModuleManifest};

    #[test]
    fn process_module_is_reported_as_process_source() {
        let entry = ModuleCatalogEntrySummary {
            slot: "search".to_owned(),
            id: "rg".to_owned(),
            manifest: ModuleManifest::process("rg", ModuleKind::Search, "v1", &["process"]),
        };

        assert_eq!(module_source(&entry), ModuleSourceTopology::Process);
    }
}
