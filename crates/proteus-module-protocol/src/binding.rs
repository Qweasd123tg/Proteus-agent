use anyhow::{Result, bail};
use std::collections::BTreeSet;

use proteus_contracts::contracts::{
    ProcessComponentExportInitialize, ProcessComponentExportRef, ProcessComponentInitialize,
};
use serde_json::Value;

use crate::{ProcessContractAuthority, process_contract_authority};

/// Explicit binding одного module export внутри process component.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessExportBinding {
    pub slot: String,
    pub module_id: String,
    pub contract_version: String,
    pub module_config: Value,
}

impl ProcessExportBinding {
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
    ) -> ProcessComponentExportInitialize {
        ProcessComponentExportInitialize::new(
            self.slot.clone(),
            self.module_id.clone(),
            self.contract_version.clone(),
            authority.composition,
            self.module_config.clone(),
            authority.host_features.iter().copied(),
        )
    }

    pub fn export_ref(&self) -> ProcessComponentExportRef {
        ProcessComponentExportRef::new(self.slot.clone(), self.module_id.clone())
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

/// Один configured process instance и полный exact set его exports.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessComponentBinding {
    pub component_id: String,
    pub exports: Vec<ProcessExportBinding>,
}

impl ProcessComponentBinding {
    pub fn new(
        component_id: impl Into<String>,
        exports: impl IntoIterator<Item = ProcessExportBinding>,
    ) -> Result<Self> {
        let binding = Self {
            component_id: component_id.into(),
            exports: exports.into_iter().collect(),
        };
        binding.validate()?;
        Ok(binding)
    }

    pub fn export(&self, target: &ProcessComponentExportRef) -> Result<&ProcessExportBinding> {
        self.exports
            .iter()
            .find(|binding| binding.slot == target.slot && binding.module_id == target.module_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "component {:?} has no export {}/{}",
                    self.component_id,
                    target.slot,
                    target.module_id
                )
            })
    }

    pub(crate) fn initialize(&self) -> Result<ProcessComponentInitialize> {
        let exports = self
            .exports
            .iter()
            .map(|binding| Ok(binding.initialize(*binding.authority()?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(ProcessComponentInitialize::new(
            self.component_id.clone(),
            exports,
        ))
    }

    fn validate(&self) -> Result<()> {
        if self.component_id.trim().is_empty() {
            bail!("process component id must not be empty");
        }
        if self.exports.is_empty() {
            bail!(
                "process component {:?} must declare exports",
                self.component_id
            );
        }
        let mut seen = BTreeSet::new();
        for export in &self.exports {
            export.validate()?;
            export.authority()?;
            if !seen.insert((export.slot.as_str(), export.module_id.as_str())) {
                bail!(
                    "process component {:?} repeats export {}/{}",
                    self.component_id,
                    export.slot,
                    export.module_id
                );
            }
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
        ProcessExportBinding::new("search", "", PROCESS_SEARCH_CONTRACT_VERSION, json!({}))
            .expect_err("empty module id must fail");
    }

    #[test]
    fn binding_requires_an_admitted_contract() {
        let binding = ProcessExportBinding::new("search", "fixture", "v999", json!({}))
            .expect("binding shape");

        binding.authority().expect_err("unknown contract must fail");
    }

    #[test]
    fn binding_requires_an_object_config() {
        ProcessExportBinding::new(
            "search",
            "fixture",
            PROCESS_SEARCH_CONTRACT_VERSION,
            json!(["not", "an", "object"]),
        )
        .expect_err("module config must be an object");
    }

    #[test]
    fn component_binding_rejects_duplicate_exports() {
        let first = ProcessExportBinding::new(
            "search",
            "fixture",
            PROCESS_SEARCH_CONTRACT_VERSION,
            json!({}),
        )
        .expect("first");
        let error = ProcessComponentBinding::new("fixture", [first.clone(), first])
            .expect_err("duplicate export must fail");

        assert!(error.to_string().contains("repeats export"));
    }
}
