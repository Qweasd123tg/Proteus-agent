use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use proteus_contracts::contracts::ProcessComponentManifest;
use proteus_process_host::{NewlineJsonFraming, ProcessTransport};

use crate::{ProcessComponentBinding, handshake::validate_manifest};

use super::wire::{COMPONENT_PROTOCOL_V3, host_id, initialize_request};
use super::{ComponentFrame, WireDirection, parse_component_frame, parse_wire_id};

pub(crate) fn initialize_transport(
    transport: &mut ProcessTransport<NewlineJsonFraming>,
    binding: &ProcessComponentBinding,
    generation: u64,
    timeout: Duration,
) -> Result<ProcessComponentManifest> {
    let started = Instant::now();
    let initialize = binding.initialize()?;
    let params = serde_json::to_value(initialize)?;
    let dispatch = transport
        .frame_writer()
        .queue_control_frame(initialize_request(generation, params))
        .context("failed to write component-v3 initialize request")?;
    let Some(write_result) = dispatch.wait_timeout(timeout.saturating_sub(started.elapsed()))
    else {
        // The independent lifecycle owner interrupts a writer blocked in the
        // child pipe. A queued Shutdown alone cannot unblock that write.
        transport.terminate()?;
        bail!("component-v3 initialize request timed out while writing after {timeout:?}");
    };
    write_result.context("failed to write component-v3 initialize request")?;
    let Some(remaining) = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
    else {
        bail!("component-v3 initialize request timed out after {timeout:?}");
    };
    let frame = transport
        .recv_frame(remaining)
        .map_err(anyhow::Error::from)
        .context("component-v3 initialize request failed")?;
    let ComponentFrame::Response { id, result } = parse_component_frame(frame)
        .context("invalid component-v3 initialize response envelope")?
    else {
        bail!("component-v3 initialize must receive one terminal response");
    };
    let wire_id = parse_wire_id(&id).context("invalid component-v3 initialize response id")?;
    if wire_id.direction != WireDirection::Host
        || wire_id.generation != generation
        || wire_id.sequence != 0
        || id != host_id(generation, 0)
    {
        bail!("component-v3 initialize response id {id:?} did not match generation {generation}");
    }
    let manifest: ProcessComponentManifest =
        serde_json::from_value(result.map_err(anyhow::Error::from)?)
            .context("component-v3 initialize returned an invalid manifest")?;
    validate_manifest(&manifest, binding, COMPONENT_PROTOCOL_V3)?;
    Ok(manifest)
}
