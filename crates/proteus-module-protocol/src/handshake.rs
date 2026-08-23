use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use proteus_contracts::contracts::{ProcessComponentExportManifest, ProcessComponentManifest};

use crate::{ProcessComponentBinding, ProcessContractAuthority, ProcessExportBinding};

pub(crate) fn validate_manifest(
    manifest: &ProcessComponentManifest,
    binding: &ProcessComponentBinding,
    expected_protocol_version: &str,
) -> Result<()> {
    if manifest.protocol_version != expected_protocol_version {
        bail!(
            "process component protocol mismatch: expected {:?}, got {:?}",
            expected_protocol_version,
            manifest.protocol_version
        );
    }
    if manifest.component_id != binding.component_id {
        bail!(
            "process component id mismatch: expected {:?}, got {:?}",
            binding.component_id,
            manifest.component_id
        );
    }

    let expected = binding
        .exports
        .iter()
        .map(|export| ((export.slot.as_str(), export.module_id.as_str()), export))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    for export in &manifest.exports {
        let key = (export.slot.as_str(), export.module_id.as_str());
        if !seen.insert(key) {
            bail!(
                "process component manifest repeats export {}/{}",
                export.slot,
                export.module_id
            );
        }
        let Some(binding) = expected.get(&key) else {
            bail!(
                "process component returned undeclared export {}/{}",
                export.slot,
                export.module_id
            );
        };
        validate_export_manifest(export, binding, *binding.authority()?)?;
    }
    if seen.len() != expected.len() {
        let missing = expected
            .keys()
            .find(|key| !seen.contains(*key))
            .expect("different set sizes imply a missing export");
        bail!(
            "process component manifest omitted export {}/{}",
            missing.0,
            missing.1
        );
    }
    Ok(())
}

fn validate_export_manifest(
    manifest: &ProcessComponentExportManifest,
    binding: &ProcessExportBinding,
    authority: ProcessContractAuthority,
) -> Result<()> {
    if manifest.contract_version != binding.contract_version {
        bail!(
            "process module contract mismatch: expected {:?}, got {:?}",
            binding.contract_version,
            manifest.contract_version
        );
    }
    if manifest.composition != authority.composition {
        bail!(
            "process module composition mismatch: expected {:?}, got {:?}",
            authority.composition,
            manifest.composition
        );
    }
    validate_module_features(
        &manifest.module_features,
        authority.host_features,
        authority.required_features,
    )
}

fn validate_module_features(
    module_features: &[String],
    offered: &[&str],
    required: &[&str],
) -> Result<()> {
    let mut unique = BTreeSet::new();
    for feature in module_features {
        if feature.trim().is_empty() {
            bail!("process module feature must not be empty");
        }
        if !unique.insert(feature.as_str()) {
            bail!("process module feature {feature:?} is duplicated");
        }
        if !offered.contains(&feature.as_str()) {
            bail!("process module acknowledged unoffered feature {feature:?}");
        }
    }
    for feature in required {
        if !unique.contains(feature) {
            bail!("process module does not support required feature {feature:?}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_protocol_feature_must_be_acknowledged() {
        let error = validate_module_features(&[], &["branch_state"], &["branch_state"])
            .expect_err("required feature must not be optional");

        assert!(error.to_string().contains("required feature"));
        validate_module_features(
            &["branch_state".to_owned()],
            &["branch_state"],
            &["branch_state"],
        )
        .expect("acknowledged required feature");
    }

    #[test]
    fn optional_protocol_feature_may_be_omitted() {
        validate_module_features(&[], &["progress_v2"], &[])
            .expect("optional feature may be omitted");
    }

    #[test]
    fn protocol_features_are_unique() {
        let features = ["progress_v2".to_owned(), "progress_v2".to_owned()];
        let error = validate_module_features(&features, &["progress_v2"], &[])
            .expect_err("duplicate feature acknowledgement must fail");

        assert!(error.to_string().contains("duplicated"));
    }
}
