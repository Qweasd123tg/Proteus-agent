use crate::domain::ModuleKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoreSlotSelection {
    ProviderConfig,
    ModulesConfig,
    OrderedModulesConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CoreSlotDescriptor {
    pub kind: ModuleKind,
    pub title: &'static str,
    pub responsibility: &'static str,
    pub category: &'static str,
    pub order: u32,
    pub required: bool,
    pub selection: CoreSlotSelection,
}

pub(crate) const CORE_SLOT_DESCRIPTORS: [CoreSlotDescriptor; 7] = [
    CoreSlotDescriptor {
        kind: ModuleKind::Hook,
        title: "Hooks",
        responsibility: "Applies ordered typed execution contributions.",
        category: "pipeline",
        order: 6,
        required: false,
        selection: CoreSlotSelection::OrderedModulesConfig,
    },
    CoreSlotDescriptor {
        kind: ModuleKind::Workflow,
        title: "Workflow",
        responsibility: "Controls the agent turn loop: planning, model calls, tool calls, review.",
        category: "orchestrator",
        order: 0,
        required: true,
        selection: CoreSlotSelection::ModulesConfig,
    },
    CoreSlotDescriptor {
        kind: ModuleKind::Context,
        title: "Context",
        responsibility: "Builds context chunks before model calls.",
        category: "pipeline",
        order: 1,
        required: true,
        selection: CoreSlotSelection::ModulesConfig,
    },
    CoreSlotDescriptor {
        kind: ModuleKind::Compactor,
        title: "Compactor",
        responsibility: "Compacts long histories before model requests.",
        category: "pipeline",
        order: 2,
        required: true,
        selection: CoreSlotSelection::ModulesConfig,
    },
    CoreSlotDescriptor {
        kind: ModuleKind::ToolExposure,
        title: "Tool Exposure",
        responsibility: "Chooses which registered tools are exposed to the model.",
        category: "pipeline",
        order: 3,
        required: true,
        selection: CoreSlotSelection::ModulesConfig,
    },
    CoreSlotDescriptor {
        kind: ModuleKind::Model,
        title: "Model",
        responsibility: "Adapts canonical model requests to provider APIs.",
        category: "pipeline",
        order: 4,
        required: false,
        selection: CoreSlotSelection::ProviderConfig,
    },
    CoreSlotDescriptor {
        kind: ModuleKind::Policy,
        title: "Policy",
        responsibility: "Evaluates tool execution and approval requirements.",
        category: "pipeline",
        order: 5,
        required: true,
        selection: CoreSlotSelection::ModulesConfig,
    },
];

pub(crate) fn core_slot_descriptor_by_id(id: &str) -> Option<&'static CoreSlotDescriptor> {
    CORE_SLOT_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.kind.as_str() == id)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn descriptors_cover_each_behavior_slot_once() {
        let kinds = CORE_SLOT_DESCRIPTORS
            .iter()
            .map(|descriptor| descriptor.kind)
            .collect::<BTreeSet<_>>();
        let behavior_slots = ModuleKind::ALL
            .into_iter()
            .filter(|kind| *kind != ModuleKind::Tool)
            .collect::<BTreeSet<_>>();

        assert_eq!(kinds, behavior_slots);
        assert!(!kinds.contains(&ModuleKind::Tool));
        assert!(core_slot_descriptor_by_id("tool").is_none());
    }
}
