use proteus_contracts::contracts::{
    PROCESS_COMPACTOR_CONTRACT_VERSION, PROCESS_COMPACTOR_METHOD, PROCESS_SEARCH_CONTRACT_VERSION,
    PROCESS_SEARCH_METHOD, ProcessModuleComposition,
};

const NO_HOST_METHODS: &[&str] = &[];
const NO_PROTOCOL_FEATURES: &[&str] = &[];
const SEARCH_METHODS: &[&str] = &[PROCESS_SEARCH_METHOD];
const COMPACTOR_METHODS: &[&str] = &[PROCESS_COMPACTOR_METHOD];

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
        host_methods: NO_HOST_METHODS,
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
}
