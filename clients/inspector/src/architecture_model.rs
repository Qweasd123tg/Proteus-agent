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

#[derive(Clone, Debug)]
pub(crate) struct ContributionChip {
    pub(crate) key: String,
    pub(crate) text: String,
    pub(crate) state: ContributionState,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ContributionState {
    Active,
    Available,
    Inactive,
}

impl ContributionState {
    pub(crate) fn chip_class(self) -> &'static str {
        match self {
            Self::Active => "contribution-chip active",
            Self::Available => "contribution-chip available",
            Self::Inactive => "contribution-chip inactive",
        }
    }
}

pub(crate) fn slot_views(snapshot: &TopologySnapshot) -> Vec<SlotView> {
    let mut views = snapshot
        .slots
        .iter()
        // Старые topology snapshots представляли ToolRegistry pseudo-slot-ом.
        // Это runtime node, а не заменяемый module slot, поэтому не показываем
        // его среди slots даже при чтении legacy snapshot-а.
        .filter(|slot| slot.id != "tool")
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
                category: slot_category(slot),
                active_module,
                alternatives,
                slot: slot.clone(),
            }
        })
        .collect::<Vec<_>>();
    views.sort_by(|left, right| {
        slot_order(&left.slot)
            .cmp(&slot_order(&right.slot))
            .then_with(|| left.slot.id.cmp(&right.slot.id))
    });
    views
}

pub(crate) fn module_source_label(source: &TopologyModuleSource) -> String {
    match source.kind.as_str() {
        "plugin" => source
            .name
            .as_deref()
            .map(|name| format!("plugin:{name}"))
            .unwrap_or_else(|| "plugin".to_owned()),
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

pub(crate) fn plugin_contributions(
    snapshot: &TopologySnapshot,
    plugin: &TopologyPlugin,
) -> Vec<ContributionChip> {
    let mut chips = Vec::new();
    for module in &plugin.provides.modules {
        let state = snapshot
            .modules
            .iter()
            .find(|candidate| candidate.slot == module.slot && candidate.id == module.id)
            .map(|candidate| {
                if candidate.active {
                    ContributionState::Active
                } else {
                    ContributionState::Available
                }
            })
            .unwrap_or(ContributionState::Inactive);
        let suffix = match state {
            ContributionState::Active => "active",
            ContributionState::Available => "available",
            ContributionState::Inactive => "provided",
        };
        chips.push(ContributionChip {
            key: format!("module:{}:{}", module.slot, module.id),
            text: format!("{}/{} · {suffix}", module.slot, module.id),
            state,
        });
    }
    for tool in &plugin.provides.tools {
        let (state, suffix) = snapshot
            .tools
            .iter()
            .find(|candidate| candidate.name == tool.name)
            .map(
                |candidate| match (candidate.enabled, candidate.registered) {
                    (true, true) => (ContributionState::Active, "enabled"),
                    (false, true) => (ContributionState::Available, "registered"),
                    (true, false) => (ContributionState::Inactive, "enabled, не registered"),
                    (false, false) => (ContributionState::Inactive, "disabled"),
                },
            )
            .unwrap_or((ContributionState::Inactive, "provided"));
        chips.push(ContributionChip {
            key: format!("tool:{}", tool.name),
            text: format!("tool {} · {suffix}", tool.name),
            state,
        });
    }
    for provider in &plugin.provides.context_providers {
        chips.push(ContributionChip {
            key: format!("context:{provider}"),
            text: format!("context {provider} → slot:context"),
            state: ContributionState::Active,
        });
    }
    chips
}

pub(crate) fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

/// Fallback на случай старого backend без `category`/`order` в snapshot.
fn slot_category(slot: &TopologySlot) -> String {
    if !slot.category.trim().is_empty() {
        return slot.category.clone();
    }
    match slot.id.as_str() {
        "workflow" => "orchestrator",
        "context" | "compactor" | "model" | "tool_exposure" | "subagent" | "policy"
        | "renderer" => "pipeline",
        "search" | "patch" | "memory" => "backend",
        _ => "custom",
    }
    .to_owned()
}

fn slot_order(slot: &TopologySlot) -> u32 {
    if slot.order > 0 || slot.id == "workflow" {
        return slot.order;
    }
    match slot.id.as_str() {
        "workflow" => 0,
        "context" => 1,
        "compactor" => 2,
        // Держать в согласии с core/core_slots.rs: exposure идёт до model.
        "tool_exposure" => 3,
        "model" => 4,
        "policy" => 5,
        "subagent" => 6,
        "renderer" => 7,
        "search" => 8,
        "patch" => 9,
        "memory" => 10,
        _ => 100,
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
                slot("renderer", "pipeline", 7),
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

        assert_eq!(ids, ["config", "workflow", "policy", "tools", "renderer"]);
        let tools = steps
            .iter()
            .find(|step| step.id == "tools")
            .expect("tools step");
        assert_eq!(tools.label, "tools");
        assert_eq!(tools.detail, "2 registered");
        assert_eq!(tools.source, "1 enabled");
        assert!(!tools.missing);
    }

    #[test]
    fn legacy_tool_slot_is_hidden_and_does_not_duplicate_tools_step() {
        let snapshot = TopologySnapshot {
            slots: vec![
                slot("policy", "pipeline", 5),
                slot("tool", "registry", 6),
                slot("renderer", "pipeline", 8),
            ],
            tools: vec![tool("read_file", true, true)],
            ..TopologySnapshot::default()
        };

        let slots = slot_views(&snapshot);
        assert!(slots.iter().all(|view| view.slot.id != "tool"));

        let steps = pipeline_steps(&snapshot, &slots);
        assert_eq!(steps.iter().filter(|step| step.id == "tools").count(), 1);
        assert_eq!(
            steps
                .iter()
                .map(|step| step.id.as_str())
                .collect::<Vec<_>>(),
            ["config", "policy", "tools", "renderer"]
        );
    }
}
