use std::collections::{BTreeMap, BTreeSet};

use crate::core::{
    ModuleCatalogEntrySummary,
    core_slots::{CORE_SLOT_DESCRIPTORS, core_slot_descriptor_by_id},
};
use crate::domain::ModuleKind;

use super::SlotTopology;

pub(super) fn build_slots(
    catalog_entries: &[ModuleCatalogEntrySummary],
    active_modules: &BTreeMap<String, String>,
) -> Vec<SlotTopology> {
    let mut slot_ids = CORE_SLOT_DESCRIPTORS
        .iter()
        .map(|descriptor| descriptor.kind.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    slot_ids.extend(
        catalog_entries
            .iter()
            .filter(|entry| entry.slot != ModuleKind::Tool.as_str())
            .map(|entry| entry.slot.clone()),
    );

    let mut slots = slot_ids
        .into_iter()
        .map(|id| {
            let descriptor = core_slot_descriptor_by_id(&id);
            SlotTopology {
                title: descriptor
                    .map(|descriptor| descriptor.title)
                    .unwrap_or("Custom Slot")
                    .to_owned(),
                responsibility: descriptor
                    .map(|descriptor| descriptor.responsibility)
                    .unwrap_or("Custom catalog namespace without a core-owned lifecycle.")
                    .to_owned(),
                active_module: active_modules.get(&id).cloned(),
                required: descriptor.is_some_and(|descriptor| descriptor.required),
                category: descriptor
                    .map(|descriptor| descriptor.category)
                    .unwrap_or("custom")
                    .to_owned(),
                order: descriptor.map(|descriptor| descriptor.order).unwrap_or(100),
                id,
            }
        })
        .collect::<Vec<_>>();
    slots.sort_by(|left, right| {
        left.order
            .cmp(&right.order)
            .then_with(|| left.id.cmp(&right.id))
    });
    slots
}
