use crate::core::{ModuleSourceTopology, SlotTopology, TopologySnapshot};

pub(super) fn ordered_slots(snapshot: &TopologySnapshot) -> Vec<&SlotTopology> {
    // build_slots уже сортирует по slot.order; стабильная пересортировка
    // оставляет порядок snapshot-а и для legacy snapshot с order=0.
    let mut slots = snapshot.slots.iter().collect::<Vec<_>>();
    slots.sort_by_key(|slot| slot.order);
    slots
}

pub(super) fn active_module_source(
    snapshot: &TopologySnapshot,
    slot_id: &str,
    module_id: &str,
) -> String {
    snapshot
        .modules
        .iter()
        .find(|module| module.slot == slot_id && module.id == module_id)
        .map(|module| module_source_label(&module.source))
        .unwrap_or_else(|| "unknown".to_owned())
}

pub(super) fn module_source_label(source: &ModuleSourceTopology) -> String {
    match source {
        ModuleSourceTopology::Builtin => "builtin".to_owned(),
        ModuleSourceTopology::Plugin { name, .. } => format!("plugin:{name}"),
        ModuleSourceTopology::Config => "config".to_owned(),
        ModuleSourceTopology::Unknown => "unknown".to_owned(),
    }
}

pub(super) fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

pub(super) fn mermaid_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(super) fn plain_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
