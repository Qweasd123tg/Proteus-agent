use crate::types::*;

#[derive(Clone, Debug)]
pub(crate) struct SlotView {
    pub(crate) slot: TopologySlot,
    pub(crate) active_module: Option<TopologyModule>,
    pub(crate) alternatives: Vec<TopologyModule>,
}

pub(crate) fn slot_views(snapshot: &TopologySnapshot) -> Vec<SlotView> {
    let mut views = snapshot
        .slots
        .iter()
        .map(|slot| {
            let active_module = slot.active_module.as_ref().and_then(|active| {
                snapshot
                    .modules
                    .iter()
                    .find(|module| module.slot == slot.id && module.id == *active)
                    .cloned()
            });
            let mut alternatives = snapshot
                .modules
                .iter()
                .filter(|module| module.slot == slot.id && !module.active)
                .cloned()
                .collect::<Vec<_>>();
            alternatives.sort_by(|left, right| left.id.cmp(&right.id));
            SlotView {
                active_module,
                alternatives,
                slot: slot.clone(),
            }
        })
        .collect::<Vec<_>>();
    views.sort_by(|left, right| {
        left.slot
            .order
            .cmp(&right.slot.order)
            .then_with(|| left.slot.id.cmp(&right.slot.id))
    });
    views
}

pub(crate) fn module_source_label(source: &TopologyModuleSource) -> String {
    match source.kind.as_str() {
        "process" => "process".to_owned(),
        "builtin" => "builtin".to_owned(),
        "config" => "config".to_owned(),
        _ => "unknown".to_owned(),
    }
}

pub(crate) fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}
