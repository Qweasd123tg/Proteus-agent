use crate::domain::ModuleKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoreSlotSelection {
    ProviderConfig,
    ModulesConfig,
    ToolRegistry,
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

pub(crate) const CORE_SLOT_DESCRIPTORS: [CoreSlotDescriptor; 12] = [
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
        required: true,
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
    CoreSlotDescriptor {
        kind: ModuleKind::Tool,
        title: "Tools",
        responsibility: "Executable capabilities available through ToolRegistry.",
        category: "registry",
        order: 6,
        required: false,
        selection: CoreSlotSelection::ToolRegistry,
    },
    CoreSlotDescriptor {
        kind: ModuleKind::Subagent,
        title: "Subagent",
        responsibility: "Runs delegated child agent loops for workflows.",
        category: "pipeline",
        order: 7,
        required: true,
        selection: CoreSlotSelection::ModulesConfig,
    },
    CoreSlotDescriptor {
        kind: ModuleKind::Renderer,
        title: "Renderer",
        responsibility: "Renders final AgentOutput for clients/CLI.",
        category: "pipeline",
        order: 8,
        required: true,
        selection: CoreSlotSelection::ModulesConfig,
    },
    CoreSlotDescriptor {
        kind: ModuleKind::Search,
        title: "Search",
        responsibility: "Provides repository/search backend.",
        category: "backend",
        order: 9,
        required: false,
        selection: CoreSlotSelection::ModulesConfig,
    },
    CoreSlotDescriptor {
        kind: ModuleKind::Patch,
        title: "Patch",
        responsibility: "Applies structured patches to the workspace.",
        category: "backend",
        order: 10,
        required: true,
        selection: CoreSlotSelection::ModulesConfig,
    },
    CoreSlotDescriptor {
        kind: ModuleKind::Memory,
        title: "Memory",
        responsibility: "Persists and retrieves explicit memories.",
        category: "backend",
        order: 11,
        required: false,
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
    fn descriptors_cover_each_host_slot_once() {
        let kinds = CORE_SLOT_DESCRIPTORS
            .iter()
            .map(|descriptor| descriptor.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(kinds.len(), ModuleKind::ALL.len());
        assert_eq!(kinds, ModuleKind::ALL.into_iter().collect::<BTreeSet<_>>());
    }

    #[test]
    fn descriptors_have_expected_selection_owners() {
        let modules = CORE_SLOT_DESCRIPTORS
            .iter()
            .filter(|descriptor| descriptor.selection == CoreSlotSelection::ModulesConfig)
            .count();
        let providers = CORE_SLOT_DESCRIPTORS
            .iter()
            .filter(|descriptor| descriptor.selection == CoreSlotSelection::ProviderConfig)
            .count();
        let registries = CORE_SLOT_DESCRIPTORS
            .iter()
            .filter(|descriptor| descriptor.selection == CoreSlotSelection::ToolRegistry)
            .count();

        assert_eq!(modules, 10);
        assert_eq!(providers, 1);
        assert_eq!(registries, 1);
    }
}
