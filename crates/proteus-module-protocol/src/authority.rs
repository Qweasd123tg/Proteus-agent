use proteus_contracts::contracts::{
    COMPACTOR_HOST_COMPLETE_MODEL_METHOD, CONTEXT_HOST_PROVIDER_METHOD,
    CONTEXT_HOST_RECALL_MEMORY_METHOD, CONTEXT_HOST_SEARCH_METHOD,
    PROCESS_COMPACTOR_CONTRACT_VERSION, PROCESS_COMPACTOR_METHOD, PROCESS_CONTEXT_BUILD_METHOD,
    PROCESS_CONTEXT_CONTRACT_VERSION, PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION,
    PROCESS_CONTEXT_PROVIDER_METHOD, PROCESS_MEMORY_CONTRACT_VERSION, PROCESS_MEMORY_RECALL_METHOD,
    PROCESS_MEMORY_REMEMBER_METHOD, PROCESS_PATCH_APPLY_METHOD, PROCESS_PATCH_CONTRACT_VERSION,
    PROCESS_POLICY_CONTRACT_VERSION, PROCESS_POLICY_EVALUATE_METHOD,
    PROCESS_POLICY_VISIBILITY_METHOD, PROCESS_RENDERER_CONTRACT_VERSION,
    PROCESS_RENDERER_RENDER_METHOD, PROCESS_SEARCH_CONTRACT_VERSION, PROCESS_SEARCH_METHOD,
    PROCESS_TOOL_CONTRACT_VERSION, PROCESS_TOOL_EXPOSURE_CONTRACT_VERSION,
    PROCESS_TOOL_EXPOSURE_SELECT_METHOD, PROCESS_TOOL_INVOKE_METHOD, PROCESS_TOOL_LIST_METHOD,
    PROCESS_WORKFLOW_CONTRACT_VERSION, PROCESS_WORKFLOW_METHOD, ProcessModuleComposition,
    WORKFLOW_HOST_BUILD_CONTEXT_METHOD, WORKFLOW_HOST_COMPACT_HISTORY_METHOD,
    WORKFLOW_HOST_COMPLETE_MODEL_METHOD, WORKFLOW_HOST_EMIT_EVENT_METHOD,
    WORKFLOW_HOST_EXECUTE_TOOL_METHOD, WORKFLOW_HOST_EXECUTE_TOOLS_METHOD,
    WORKFLOW_HOST_RUNTIME_STATUS_METHOD, WORKFLOW_HOST_SELECT_TOOLS_METHOD,
    WORKFLOW_HOST_VISIBLE_TOOLS_METHOD,
};

const NO_HOST_METHODS: &[&str] = &[];
const NO_PROTOCOL_FEATURES: &[&str] = &[];
const SEARCH_METHODS: &[&str] = &[PROCESS_SEARCH_METHOD];
const COMPACTOR_METHODS: &[&str] = &[PROCESS_COMPACTOR_METHOD];
const COMPACTOR_HOST_METHODS: &[&str] = &[COMPACTOR_HOST_COMPLETE_MODEL_METHOD];
const MEMORY_METHODS: &[&str] = &[PROCESS_MEMORY_REMEMBER_METHOD, PROCESS_MEMORY_RECALL_METHOD];
const PATCH_METHODS: &[&str] = &[PROCESS_PATCH_APPLY_METHOD];
const TOOL_EXPOSURE_METHODS: &[&str] = &[PROCESS_TOOL_EXPOSURE_SELECT_METHOD];
const POLICY_METHODS: &[&str] = &[
    PROCESS_POLICY_EVALUATE_METHOD,
    PROCESS_POLICY_VISIBILITY_METHOD,
];
const RENDERER_METHODS: &[&str] = &[PROCESS_RENDERER_RENDER_METHOD];
const CONTEXT_METHODS: &[&str] = &[PROCESS_CONTEXT_BUILD_METHOD];
const CONTEXT_HOST_METHODS: &[&str] = &[
    CONTEXT_HOST_SEARCH_METHOD,
    CONTEXT_HOST_RECALL_MEMORY_METHOD,
    CONTEXT_HOST_PROVIDER_METHOD,
];
const CONTEXT_PROVIDER_METHODS: &[&str] = &[PROCESS_CONTEXT_PROVIDER_METHOD];
const TOOL_METHODS: &[&str] = &[PROCESS_TOOL_LIST_METHOD, PROCESS_TOOL_INVOKE_METHOD];
const WORKFLOW_METHODS: &[&str] = &[PROCESS_WORKFLOW_METHOD];
const WORKFLOW_HOST_METHODS: &[&str] = &[
    WORKFLOW_HOST_RUNTIME_STATUS_METHOD,
    WORKFLOW_HOST_BUILD_CONTEXT_METHOD,
    WORKFLOW_HOST_COMPLETE_MODEL_METHOD,
    WORKFLOW_HOST_COMPACT_HISTORY_METHOD,
    WORKFLOW_HOST_VISIBLE_TOOLS_METHOD,
    WORKFLOW_HOST_SELECT_TOOLS_METHOD,
    WORKFLOW_HOST_EXECUTE_TOOL_METHOD,
    WORKFLOW_HOST_EXECUTE_TOOLS_METHOD,
    WORKFLOW_HOST_EMIT_EVENT_METHOD,
];

/// One host-defined process contract and its complete callback authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessContractAuthority {
    pub slot: &'static str,
    pub contract_version: &'static str,
    pub composition: ProcessModuleComposition,
    pub module_methods: &'static [&'static str],
    pub host_methods: &'static [&'static str],
    pub host_features: &'static [&'static str],
    pub required_features: &'static [&'static str],
}

impl ProcessContractAuthority {
    pub fn allows_host_method(self, method: &str) -> bool {
        self.host_methods.contains(&method)
    }

    pub fn allows_module_method(self, method: &str) -> bool {
        self.module_methods.contains(&method)
    }
}

/// Single source of truth for process contracts already admitted to the v1
/// host runtime. More slots are added only together with their contract and
/// conformance evidence.
pub const PROCESS_CONTRACT_AUTHORITIES: &[ProcessContractAuthority] = &[
    ProcessContractAuthority {
        slot: "search",
        contract_version: PROCESS_SEARCH_CONTRACT_VERSION,
        composition: ProcessModuleComposition::SelectOne,
        module_methods: SEARCH_METHODS,
        host_methods: NO_HOST_METHODS,
        host_features: NO_PROTOCOL_FEATURES,
        required_features: NO_PROTOCOL_FEATURES,
    },
    ProcessContractAuthority {
        slot: "compactor",
        contract_version: PROCESS_COMPACTOR_CONTRACT_VERSION,
        composition: ProcessModuleComposition::SelectOne,
        module_methods: COMPACTOR_METHODS,
        host_methods: COMPACTOR_HOST_METHODS,
        host_features: NO_PROTOCOL_FEATURES,
        required_features: NO_PROTOCOL_FEATURES,
    },
    ProcessContractAuthority {
        slot: "memory",
        contract_version: PROCESS_MEMORY_CONTRACT_VERSION,
        composition: ProcessModuleComposition::SelectOne,
        module_methods: MEMORY_METHODS,
        host_methods: NO_HOST_METHODS,
        host_features: NO_PROTOCOL_FEATURES,
        required_features: NO_PROTOCOL_FEATURES,
    },
    ProcessContractAuthority {
        slot: "patch",
        contract_version: PROCESS_PATCH_CONTRACT_VERSION,
        composition: ProcessModuleComposition::SelectOne,
        module_methods: PATCH_METHODS,
        host_methods: NO_HOST_METHODS,
        host_features: NO_PROTOCOL_FEATURES,
        required_features: NO_PROTOCOL_FEATURES,
    },
    ProcessContractAuthority {
        slot: "tool_exposure",
        contract_version: PROCESS_TOOL_EXPOSURE_CONTRACT_VERSION,
        composition: ProcessModuleComposition::SelectOne,
        module_methods: TOOL_EXPOSURE_METHODS,
        host_methods: NO_HOST_METHODS,
        host_features: NO_PROTOCOL_FEATURES,
        required_features: NO_PROTOCOL_FEATURES,
    },
    ProcessContractAuthority {
        slot: "policy",
        contract_version: PROCESS_POLICY_CONTRACT_VERSION,
        composition: ProcessModuleComposition::SelectOne,
        module_methods: POLICY_METHODS,
        host_methods: NO_HOST_METHODS,
        host_features: NO_PROTOCOL_FEATURES,
        required_features: NO_PROTOCOL_FEATURES,
    },
    ProcessContractAuthority {
        slot: "renderer",
        contract_version: PROCESS_RENDERER_CONTRACT_VERSION,
        composition: ProcessModuleComposition::SelectOne,
        module_methods: RENDERER_METHODS,
        host_methods: NO_HOST_METHODS,
        host_features: NO_PROTOCOL_FEATURES,
        required_features: NO_PROTOCOL_FEATURES,
    },
    ProcessContractAuthority {
        slot: "context",
        contract_version: PROCESS_CONTEXT_CONTRACT_VERSION,
        composition: ProcessModuleComposition::SelectOne,
        module_methods: CONTEXT_METHODS,
        host_methods: CONTEXT_HOST_METHODS,
        host_features: NO_PROTOCOL_FEATURES,
        required_features: NO_PROTOCOL_FEATURES,
    },
    ProcessContractAuthority {
        slot: "context_provider",
        contract_version: PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION,
        composition: ProcessModuleComposition::OrderedMany,
        module_methods: CONTEXT_PROVIDER_METHODS,
        host_methods: NO_HOST_METHODS,
        host_features: NO_PROTOCOL_FEATURES,
        required_features: NO_PROTOCOL_FEATURES,
    },
    ProcessContractAuthority {
        slot: "tool",
        contract_version: PROCESS_TOOL_CONTRACT_VERSION,
        composition: ProcessModuleComposition::OrderedMany,
        module_methods: TOOL_METHODS,
        host_methods: NO_HOST_METHODS,
        host_features: NO_PROTOCOL_FEATURES,
        required_features: NO_PROTOCOL_FEATURES,
    },
    ProcessContractAuthority {
        slot: "workflow",
        contract_version: PROCESS_WORKFLOW_CONTRACT_VERSION,
        composition: ProcessModuleComposition::SelectOne,
        module_methods: WORKFLOW_METHODS,
        host_methods: WORKFLOW_HOST_METHODS,
        host_features: NO_PROTOCOL_FEATURES,
        required_features: NO_PROTOCOL_FEATURES,
    },
];

pub fn process_contract_authority(
    slot: &str,
    contract_version: &str,
) -> Option<&'static ProcessContractAuthority> {
    PROCESS_CONTRACT_AUTHORITIES
        .iter()
        .find(|authority| authority.slot == slot && authority.contract_version == contract_version)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn admitted_contract_keys_are_unique() {
        let keys = PROCESS_CONTRACT_AUTHORITIES
            .iter()
            .map(|authority| (authority.slot, authority.contract_version))
            .collect::<BTreeSet<_>>();

        assert_eq!(keys.len(), PROCESS_CONTRACT_AUTHORITIES.len());
        for authority in PROCESS_CONTRACT_AUTHORITIES {
            assert!(
                authority
                    .required_features
                    .iter()
                    .all(|feature| authority.host_features.contains(feature)),
                "required feature must also be offered by {}/{}",
                authority.slot,
                authority.contract_version
            );
        }
    }

    #[test]
    fn search_authority_is_module_id_independent() {
        let authority = process_contract_authority("search", PROCESS_SEARCH_CONTRACT_VERSION)
            .expect("search authority");

        assert_eq!(authority.composition, ProcessModuleComposition::SelectOne);
        assert_eq!(authority.module_methods, [PROCESS_SEARCH_METHOD]);
        assert!(authority.host_methods.is_empty());
    }

    #[test]
    fn workflow_authority_is_complete_and_module_id_independent() {
        let authority = process_contract_authority("workflow", PROCESS_WORKFLOW_CONTRACT_VERSION)
            .expect("workflow authority");

        assert_eq!(authority.composition, ProcessModuleComposition::SelectOne);
        assert_eq!(authority.module_methods, [PROCESS_WORKFLOW_METHOD]);
        assert_eq!(authority.host_methods, WORKFLOW_HOST_METHODS);
    }
}
