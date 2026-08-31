use crate::types::*;

#[derive(Clone, Debug)]
pub(crate) struct SlotView {
    pub(crate) slot: TopologySlot,
    pub(crate) category: String,
    pub(crate) active_module: Option<TopologyModule>,
    pub(crate) alternatives: Vec<TopologyModule>,
}

#[derive(Clone, Debug)]
pub(crate) struct PipelineStep {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) source: String,
    pub(crate) missing: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct BackendView {
    pub(crate) slot_id: String,
    pub(crate) active_label: String,
    pub(crate) source: String,
    pub(crate) role: &'static str,
    pub(crate) used_by: Vec<String>,
    pub(crate) missing: bool,
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
                category: slot.category.clone(),
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

pub(crate) fn pipeline_steps(snapshot: &TopologySnapshot, slots: &[SlotView]) -> Vec<PipelineStep> {
    let registered = snapshot.tools.iter().filter(|tool| tool.registered).count();
    let enabled = snapshot.tools.iter().filter(|tool| tool.enabled).count();
    let mut steps = vec![PipelineStep {
        id: "config".to_owned(),
        label: "config".to_owned(),
        detail: non_empty(&snapshot.profile, "default"),
        source: snapshot.permission_mode.to_lowercase(),
        missing: false,
    }];

    for view in slots
        .iter()
        .filter(|view| matches!(view.category.as_str(), "orchestrator" | "pipeline"))
    {
        let detail = if view.slot.id == "model" {
            snapshot
                .model
                .as_ref()
                .map(|model| format!("{}/{}", model.provider, model.name))
                .or_else(|| view.slot.active_module.clone())
        } else {
            view.slot.active_module.clone()
        };
        let missing = detail.is_none();
        steps.push(PipelineStep {
            id: view.slot.id.clone(),
            label: view.slot.id.clone(),
            detail: detail.unwrap_or_else(|| "не выбран".to_owned()),
            source: view
                .active_module
                .as_ref()
                .map(|module| module_source_label(&module.source))
                .unwrap_or_else(|| {
                    if missing {
                        "missing".to_owned()
                    } else {
                        "config".to_owned()
                    }
                }),
            missing,
        });
    }

    let tools_step = PipelineStep {
        id: "tools".to_owned(),
        label: "tools".to_owned(),
        detail: format!("{registered} registered"),
        source: format!("{enabled} enabled"),
        missing: registered == 0,
    };
    let tools_index = steps
        .iter()
        .position(|step| step.id == "policy")
        .map_or(steps.len(), |policy_index| policy_index + 1);
    steps.insert(tools_index, tools_step);

    steps
}

pub(crate) fn backend_views(snapshot: &TopologySnapshot, slots: &[SlotView]) -> Vec<BackendView> {
    slots
        .iter()
        .filter(|view| matches!(view.category.as_str(), "backend" | "post_turn"))
        .map(|view| {
            let target = format!("slot:{}", view.slot.id);
            let mut used_by = snapshot
                .edges
                .iter()
                .filter(|edge| edge.kind == "uses" && edge.to == target)
                .filter_map(|edge| edge.from.strip_prefix("tool:").map(str::to_owned))
                .collect::<Vec<_>>();
            used_by.sort();
            used_by.dedup();
            BackendView {
                slot_id: view.slot.id.clone(),
                active_label: view
                    .slot
                    .active_module
                    .clone()
                    .unwrap_or_else(|| "не выбран".to_owned()),
                source: view
                    .active_module
                    .as_ref()
                    .map(|module| module_source_label(&module.source))
                    .unwrap_or_else(|| "-".to_owned()),
                role: if view.category == "post_turn" {
                    "после turn"
                } else {
                    "backend для tools"
                },
                used_by,
                missing: view.slot.active_module.is_none(),
            }
        })
        .collect()
}

pub(crate) fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(id: &str, category: &str, order: u32) -> TopologySlot {
        TopologySlot {
            id: id.to_owned(),
            active_module: Some(format!("{id}_module")),
            category: category.to_owned(),
            order,
            ..TopologySlot::default()
        }
    }

    fn tool(name: &str, registered: bool, enabled: bool) -> TopologyTool {
        TopologyTool {
            name: name.to_owned(),
            registered,
            enabled,
            ..TopologyTool::default()
        }
    }

    #[test]
    fn pipeline_derives_one_tools_step_from_inventory_after_policy() {
        let snapshot = TopologySnapshot {
            slots: vec![
                slot("workflow", "orchestrator", 0),
                slot("policy", "pipeline", 5),
            ],
            tools: vec![
                tool("read_file", true, true),
                tool("write_file", true, false),
                tool("provided_only", false, false),
            ],
            ..TopologySnapshot::default()
        };

        let slots = slot_views(&snapshot);
        let steps = pipeline_steps(&snapshot, &slots);
        let ids = steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, ["config", "workflow", "policy", "tools"]);
        let tools = steps
            .iter()
            .find(|step| step.id == "tools")
            .expect("tools step");
        assert_eq!(tools.label, "tools");
        assert_eq!(tools.detail, "2 registered");
        assert_eq!(tools.source, "1 enabled");
        assert!(!tools.missing);
    }

}
