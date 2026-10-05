use std::{path::PathBuf, sync::mpsc, thread, time::Duration};

use anyhow::{Result, ensure};
use proteus_contracts::contracts::{PROCESS_SEARCH_CONTRACT_VERSION, PROCESS_SEARCH_METHOD};
use proteus_process_host::{NewlineJsonFraming, ProcessSpec, ProcessTransport};
use serde_json::{Value, json};

use super::*;
use crate::v3::{
    config::ComponentBrokerOptions,
    handshake::initialize_transport,
    invocation::{InvocationRef, NoAsyncHostRequests},
    notification::NotificationSink,
    pending::{PendingInvocation, TerminalSender, WorkerGeneration},
};
use crate::{ProcessComponentBinding, ProcessExportBinding};

#[test]
fn cancel_tree_defers_admission_failure_and_next_generation_remains_usable() -> Result<()> {
    let binding = ProcessComponentBinding::new(
        "cancel-tree",
        [ProcessExportBinding::new(
            "search",
            "fixture.search",
            PROCESS_SEARCH_CONTRACT_VERSION,
            json!({}),
        )?],
    )?;
    let options = ComponentBrokerOptions {
        max_active_roots: 3,
        max_active_total: 5,
        reserved_nested: 2,
        max_active_nested: 2,
        max_callback_depth: 2,
        max_outbound_frame_bytes: 1024 * 1024,
        ..ComponentBrokerOptions::default()
    };
    options.validate()?;
    // This child never reads. The first frame exceeds pipe capacity, giving
    // the writer a deterministic barrier while both nested frames stay queued.
    let spec = ProcessSpec::new("python3").args(["-c", "import time; time.sleep(5)"]);
    let transport = ProcessTransport::spawn_with_limits(
        &spec,
        NewlineJsonFraming::default(),
        options.transport_limits(),
    )?;
    let writer = transport.frame_writer();
    let blocked = writer.queue_frame(json!({"barrier": "x".repeat(512 * 1024)}))?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while !blocked.is_started() {
        ensure!(
            Instant::now() < deadline,
            "writer did not claim the barrier frame"
        );
        thread::yield_now();
    }
    ensure!(
        !blocked.is_written(),
        "no-read child unexpectedly accepted the large frame"
    );

    let (control_tx, _) = mpsc::sync_channel(options.control_command_capacity);
    let mut state = LoopState::new(spec, binding.clone(), options, control_tx);
    state.worker = Some(WorkerGeneration {
        pid: transport.pid(),
        transport,
        manifest: proteus_contracts::contracts::ProcessComponentManifest {
            protocol_version: "v3".into(),
            component_id: binding.component_id.clone(),
            exports: vec![],
        },
    });
    let (mut parent, parent_result) = pending(&binding, &options, "h:1:1", None, 1, json!({}))?;
    parent.active = true;
    parent.dispatch = Some(blocked);
    state.pending.insert(parent.invocation.id.clone(), parent);
    state.active_roots = 1;

    let mut nested_results = Vec::new();
    for id in ["h:1:2", "h:1:3"] {
        let (mut child, terminal) = pending(&binding, &options, id, Some("h:1:1"), 1, json!({}))?;
        let dispatch = writer.queue_frame(json!({"nested": id}))?;
        ensure!(
            !dispatch.is_started(),
            "nested frame passed the blocked writer"
        );
        child.active = true;
        child.dispatch = Some(dispatch);
        state.pending.insert(id.to_owned(), child);
        state.active_nested += 1;
        nested_results.push(terminal);
    }
    let (queued, queued_result) = pending(
        &binding,
        &options,
        "h:1:4",
        None,
        1,
        json!({"oversized": "x".repeat(2 * 1024 * 1024)}),
    )?;
    state.pending.insert(queued.invocation.id.clone(), queued);
    state.queued_roots.push_back("h:1:4".into());
    ensure!(!state.can_activate_root());

    state.cancel("h:1:1", 1, CancelCause::User)?;
    assert_single_terminal(&parent_result, InvocationTerminal::Canceled)?;
    for terminal in nested_results {
        assert_single_terminal(&terminal, InvocationTerminal::Canceled)?;
    }
    assert_single_terminal(
        &queued_result,
        InvocationTerminal::ComponentLost(ComponentFailure::Resource),
    )?;
    ensure!(state.generation == 2 && state.pending.is_empty() && state.worker.is_none());

    // Exercise another invocation on the same state after reset, using a real
    // v3 handshake and response rather than only checking empty counters.
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multiplex_worker.py");
    let spec = ProcessSpec::new("python3").arg(fixture.to_string_lossy());
    let mut transport = ProcessTransport::spawn_with_limits(
        &spec,
        NewlineJsonFraming::default(),
        options.transport_limits(),
    )?;
    let manifest = initialize_transport(&mut transport, &binding, 2, Duration::from_secs(1))?;
    state.worker = Some(WorkerGeneration {
        pid: transport.pid(),
        transport,
        manifest,
    });
    let (replacement, replacement_result) = pending(
        &binding,
        &options,
        "h:2:1",
        None,
        2,
        json!({"op":"echo", "value":"replacement"}),
    )?;
    state
        .pending
        .insert(replacement.invocation.id.clone(), replacement);
    state.activate("h:2:1");
    let response = state
        .worker
        .as_mut()
        .expect("replacement worker")
        .transport
        .recv_frame(Duration::from_secs(1))?;
    state.handle_frame(response);
    let terminal = replacement_result.recv_timeout(Duration::from_secs(1))?;
    ensure!(
        matches!(terminal, InvocationTerminal::Success(value) if value["value"] == "replacement")
    );
    ensure!(matches!(
        replacement_result.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
    state
        .worker
        .take()
        .expect("replacement worker")
        .transport
        .terminate()?;
    Ok(())
}

fn assert_single_terminal(
    receiver: &mpsc::Receiver<InvocationTerminal>,
    expected: InvocationTerminal,
) -> Result<()> {
    ensure!(receiver.recv_timeout(Duration::from_secs(1))? == expected);
    ensure!(
        matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Disconnected)),
        "terminal sender remained live or delivered twice"
    );
    Ok(())
}

fn pending(
    binding: &ProcessComponentBinding,
    options: &ComponentBrokerOptions,
    id: &str,
    parent: Option<&str>,
    generation: u64,
    params: Value,
) -> Result<(PendingInvocation, mpsc::Receiver<InvocationTerminal>)> {
    let export = &binding.exports[0];
    let (terminal, receiver) = mpsc::channel();
    let (notifications, _) = NotificationSink::channel(options.notification_limits);
    Ok((
        PendingInvocation {
            invocation: InvocationRef {
                id: id.into(),
                generation,
                target: export.export_ref(),
                root_id: parent.unwrap_or(id).into(),
                parent_id: parent.map(str::to_owned),
                depth: usize::from(parent.is_some()),
                deadline: Instant::now() + Duration::from_secs(3),
            },
            method: PROCESS_SEARCH_METHOD.into(),
            params: Some(params),
            authority: *export.authority()?,
            dispatcher: std::sync::Arc::new(NoAsyncHostRequests),
            executor: None,
            terminal: Some(TerminalSender::Blocking(terminal)),
            notifications,
            dispatch: None,
            active: false,
            cancel: None,
            cancel_deadline: None,
            outstanding_callbacks: Default::default(),
        },
        receiver,
    ))
}
