use std::time::Duration;

use anyhow::{Context, Result, bail};
use proteus_contracts::contracts::ProcessComponentManifest;
use proteus_process_host::{NewlineJsonFraming, ProcessTransport};

use crate::{ProcessComponentBinding, handshake::validate_manifest};

use super::wire::{
    COMPONENT_PROTOCOL_V3, IdDirection, IncomingFrame, host_id, initialize_request, parse_frame,
    parse_id,
};

pub(crate) fn initialize_transport(
    transport: &mut ProcessTransport<NewlineJsonFraming>,
    binding: &ProcessComponentBinding,
    generation: u64,
    timeout: Duration,
) -> Result<()> {
    let initialize = binding.initialize()?;
    let params = serde_json::to_value(initialize)?;
    transport
        .send_control_frame(initialize_request(generation, params))
        .context("failed to write component-v3 initialize request")?;
    let frame = transport
        .recv_frame(timeout)
        .map_err(anyhow::Error::from)
        .context("component-v3 initialize request failed")?;
    let IncomingFrame::Response { id, result } =
        parse_frame(frame).context("invalid component-v3 initialize response envelope")?
    else {
        bail!("component-v3 initialize must receive one terminal response");
    };
    let wire_id = parse_id(&id).context("invalid component-v3 initialize response id")?;
    if wire_id.direction != IdDirection::Host
        || wire_id.generation != generation
        || wire_id.sequence != 0
        || id != host_id(generation, 0)
    {
        bail!("component-v3 initialize response id {id:?} did not match generation {generation}");
    }
    let manifest: ProcessComponentManifest =
        serde_json::from_value(result.map_err(anyhow::Error::from)?)
            .context("component-v3 initialize returned an invalid manifest")?;
    validate_manifest(&manifest, binding, COMPONENT_PROTOCOL_V3)
}
