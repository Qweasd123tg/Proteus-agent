use anyhow::{Result, bail};
use proteus_contracts::contracts::ProcessModuleInitialize;
use serde_json::Value;

use crate::{ProcessContractAuthority, process_contract_authority};

/// Explicit snapshot binding used to initialize one process module instance.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessModuleBinding {
    pub slot: String,
    pub module_id: String,
    pub contract_version: String,
    pub module_config: Value,
}

impl ProcessModuleBinding {
    pub fn new(
        slot: impl Into<String>,
        module_id: impl Into<String>,
        contract_version: impl Into<String>,
        module_config: Value,
    ) -> Result<Self> {
        let binding = Self {
            slot: slot.into(),
            module_id: module_id.into(),
            contract_version: contract_version.into(),
            module_config,
        };
        binding.validate()?;
        Ok(binding)
    }

    pub fn authority(&self) -> Result<&'static ProcessContractAuthority> {
        process_contract_authority(&self.slot, &self.contract_version).ok_or_else(|| {
            anyhow::anyhow!(
                "process contract is not admitted: slot {:?}, contract {:?}",
                self.slot,
                self.contract_version
            )
        })
    }

    pub(crate) fn initialize(
        &self,
        authority: ProcessContractAuthority,
    ) -> ProcessModuleInitialize {
        ProcessModuleInitialize::new(
            self.slot.clone(),
            self.module_id.clone(),
            self.contract_version.clone(),
            authority.composition,
            self.module_config.clone(),
            authority.host_features.iter().copied(),
        )
    }

    fn validate(&self) -> Result<()> {
        if self.slot.trim().is_empty() {
            bail!("process module slot must not be empty");
        }
        if self.module_id.trim().is_empty() {
            bail!("process module id must not be empty");
        }
        if self.contract_version.trim().is_empty() {
            bail!("process module contract version must not be empty");
        }
        if !self.module_config.is_object() {
            bail!("process module config must be an object");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use proteus_contracts::contracts::PROCESS_SEARCH_CONTRACT_VERSION;
    use serde_json::json;

    use super::*;

    #[test]
    fn binding_rejects_empty_identity() {
        ProcessModuleBinding::new("search", "", PROCESS_SEARCH_CONTRACT_VERSION, json!({}))
            .expect_err("empty module id must fail");
    }

    #[test]
    fn binding_requires_an_admitted_contract() {
        let binding = ProcessModuleBinding::new("search", "fixture", "v999", json!({}))
            .expect("binding shape");

        binding.authority().expect_err("unknown contract must fail");
    }

    #[test]
    fn binding_requires_an_object_config() {
        ProcessModuleBinding::new(
            "search",
            "fixture",
            PROCESS_SEARCH_CONTRACT_VERSION,
            json!(["not", "an", "object"]),
        )
        .expect_err("module config must be an object");
    }
}
