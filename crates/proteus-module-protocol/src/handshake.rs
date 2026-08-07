use std::{collections::BTreeSet, time::Duration};

use anyhow::{Context, Result, bail};
use proteus_contracts::contracts::{
    PROCESS_MODULE_INITIALIZE_METHOD, PROCESS_MODULE_PROTOCOL_VERSION, ProcessModuleInitialize,
    ProcessModuleManifest,
};
use proteus_process_host::{NewlineJsonFraming, ProcessSession};
use serde_json::json;

use crate::{
    ProcessContractAuthority, ProcessModuleBinding,
    envelope::{self, IncomingMessage},
};

pub(crate) fn initialize_session(
    session: &mut ProcessSession<NewlineJsonFraming>,
    initialize: &ProcessModuleInitialize,
    binding: &ProcessModuleBinding,
    authority: ProcessContractAuthority,
    timeout: Duration,
) -> Result<()> {
    let id = json!("initialize");
    session.send_frame(envelope::request(
        id.clone(),
        PROCESS_MODULE_INITIALIZE_METHOD,
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
    let manifest: ProcessModuleManifest =
        serde_json::from_value(value).context("initialize returned an invalid manifest")?;
    validate_manifest(&manifest, binding, authority)
}

fn validate_manifest(
    manifest: &ProcessModuleManifest,
    binding: &ProcessModuleBinding,
    authority: ProcessContractAuthority,
) -> Result<()> {
    if manifest.protocol_version != PROCESS_MODULE_PROTOCOL_VERSION {
        bail!(
            "process module protocol mismatch: expected {:?}, got {:?}",
            PROCESS_MODULE_PROTOCOL_VERSION,
            manifest.protocol_version
        );
    }
    if manifest.slot != binding.slot {
        bail!(
            "process module slot mismatch: expected {:?}, got {:?}",
            binding.slot,
            manifest.slot
        );
    }
    if manifest.module_id != binding.module_id {
        bail!(
            "process module id mismatch: expected {:?}, got {:?}",
            binding.module_id,
            manifest.module_id
        );
    }
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
