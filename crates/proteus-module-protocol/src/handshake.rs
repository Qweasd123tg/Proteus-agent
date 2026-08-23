use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use proteus_contracts::contracts::{
    PROCESS_COMPONENT_INITIALIZE_METHOD, PROCESS_COMPONENT_PROTOCOL_VERSION,
    ProcessComponentExportManifest, ProcessComponentInitialize, ProcessComponentManifest,
};
use proteus_process_host::{NewlineJsonFraming, ProcessSession};
use serde_json::json;

use crate::{
    ProcessComponentBinding, ProcessContractAuthority, ProcessExportBinding,
    envelope::{self, IncomingMessage},
};

pub(crate) fn initialize_session(
    session: &mut ProcessSession<NewlineJsonFraming>,
    initialize: &ProcessComponentInitialize,
    binding: &ProcessComponentBinding,
    timeout: Duration,
) -> Result<()> {
    let id = json!("initialize");
    session.send_frame(envelope::request(
        id.clone(),
        PROCESS_COMPONENT_INITIALIZE_METHOD,
        serde_json::to_value(initialize)?,
    ))?;
    let frame = session
        .recv_frame(timeout)
        .map_err(anyhow::Error::from)
        .context("initialize request failed")?;
    let IncomingMessage::Response {
        id: response_id,
        result,
    } = envelope::parse(frame).context("invalid initialize response envelope")?
    else {
        bail!("initialize must receive one terminal JSON-RPC response");
    };
    if response_id != id {
        bail!("initialize response id {response_id} did not match {id}");
    }
    let value = result.map_err(anyhow::Error::from)?;
    let manifest: ProcessComponentManifest =
        serde_json::from_value(value).context("initialize returned an invalid manifest")?;
    validate_manifest(&manifest, binding, PROCESS_COMPONENT_PROTOCOL_VERSION)
}

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
