mod multiplex_spike {
    pub mod broker;
    pub mod transport;
    pub mod wire;
}

use std::{env, path::PathBuf, thread, time::Duration};

use anyhow::{Result, bail, ensure};
use multiplex_spike::{
    broker::{
        Broker, BrokerConfig, CancelCause, ComponentLostCause, RejectReason, Terminal, TraceEvent,
    },
    wire::ExportRef,
};
use serde_json::{Value, json};

const SETTLE_TIMEOUT: Duration = Duration::from_secs(3);
const POLL_TIMEOUT: Duration = Duration::from_secs(1);

type NamedTest = (&'static str, fn() -> Result<()>);

fn main() {
    let tests: &[NamedTest] = &[
        (
            "non_rust_worker_completes_concurrent_calls_out_of_order",
            non_rust_worker_completes_concurrent_calls_out_of_order,
        ),
        (
            "nested_callback_is_reentrant_and_host_routed",
            nested_callback_is_reentrant_and_host_routed,
        ),
        (
            "targeted_cancel_preserves_sibling_and_generation",
            targeted_cancel_preserves_sibling_and_generation,
        ),
        (
            "queued_cancel_never_reaches_worker",
            queued_cancel_never_reaches_worker,
        ),
        (
            "uncooperative_cancel_resets_shared_failure_domain",
            uncooperative_cancel_resets_shared_failure_domain,
        ),
        (
            "parent_cancel_cascades_to_nested_invocation",
            parent_cancel_cascades_to_nested_invocation,
        ),
        (
            "callback_parent_and_duplicate_ids_fail_closed",
            callback_parent_and_duplicate_ids_fail_closed,
        ),
        (
            "terminal_with_live_callback_resets_generation",
            terminal_with_live_callback_resets_generation,
        ),
        (
            "late_frame_resets_generation_without_second_terminal",
            late_frame_resets_generation_without_second_terminal,
        ),
        (
            "slow_progress_consumer_cannot_block_control_lane",
            slow_progress_consumer_cannot_block_control_lane,
        ),
        (
            "process_exit_is_observed_while_progress_is_buffered",
            process_exit_is_observed_while_progress_is_buffered,
        ),
        (
            "nested_admission_has_depth_count_and_capacity_bounds",
            nested_admission_has_depth_count_and_capacity_bounds,
        ),
        (
            "queued_deadline_and_root_pending_capacity_are_bounded",
            queued_deadline_and_root_pending_capacity_are_bounded,
        ),
        (
            "receive_notification_trace_and_outbound_bytes_are_bounded",
            receive_notification_trace_and_outbound_bytes_are_bounded,
        ),
        (
            "blocked_writer_cannot_block_cancel_grace",
            blocked_writer_cannot_block_cancel_grace,
        ),
        (
            "cancel_command_wins_before_delayed_terminal",
            cancel_command_wins_before_delayed_terminal,
        ),
        (
            "host_deadline_becomes_targeted_timeout_cancel",
            host_deadline_becomes_targeted_timeout_cancel,
        ),
        (
            "sibling_parent_is_a_documented_trusted_component_limit",
            sibling_parent_is_a_documented_trusted_component_limit,
        ),
    ];

    let mut failures = Vec::new();
    for (name, test) in tests {
        match test() {
            Ok(()) => println!("ok - {name}"),
            Err(error) => {
                eprintln!("FAILED - {name}: {error:#}");
                failures.push(*name);
            }
        }
    }
    if !failures.is_empty() {
        eprintln!("{} multiplex spike test(s) failed", failures.len());
        std::process::exit(1);
    }
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multiplex_worker.py")
}

fn broker() -> Result<Broker> {
    Broker::spawn(fixture(), BrokerConfig::default())
}

fn workflow(module_id: &str) -> ExportRef {
    ExportRef::new("workflow", module_id)
}

fn expect_success(terminal: Terminal) -> Result<Value> {
    match terminal {
        Terminal::Success(value) => Ok(value),
        other => bail!("expected success, got {other:?}"),
    }
}

fn wait_for_trace(broker: &Broker, predicate: impl Fn(&TraceEvent) -> bool) -> Result<()> {
    let started = std::time::Instant::now();
    while started.elapsed() < POLL_TIMEOUT {
        if broker.trace().iter().any(&predicate) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(2));
    }
    bail!("expected trace event did not arrive: {:#?}", broker.trace())
}

fn ensure_one_terminal(broker: &Broker, id: &str) -> Result<()> {
    let count = broker
        .trace()
        .iter()
        .filter(|event| matches!(event, TraceEvent::Terminal { id: terminal_id, .. } if terminal_id == id))
        .count();
    ensure!(
        count == 1,
        "invocation {id} has {count} terminal trace records"
    );
    Ok(())
}

fn non_rust_worker_completes_concurrent_calls_out_of_order() -> Result<()> {
    let broker = broker()?;
    let slow = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"slow", "delay_ms":120}),
    )?;
    let fast = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"fast", "delay_ms":5}),
    )?;
    ensure!(
        slow.pid() == fast.pid(),
        "calls did not share one Python process"
    );
    ensure!(slow.generation() == fast.generation());

    let fast_value = expect_success(fast.wait(Duration::from_millis(300))?)?;
    ensure!(fast_value["value"] == json!("fast"));
    ensure!(
        slow.wait(Duration::from_millis(10)).is_err(),
        "slow call unexpectedly settled before the fast call"
    );
    let slow_value = expect_success(slow.wait(SETTLE_TIMEOUT)?)?;
    ensure!(slow_value["value"] == json!("slow"));
    Ok(())
}

fn nested_callback_is_reentrant_and_host_routed() -> Result<()> {
    let broker = broker()?;
    let root = broker.start(
        workflow("arbitrary.workflow.id"),
        json!({
            "op":"callback",
            "callback_input":{"op":"echo", "value":"nested"}
        }),
    )?;
    let root_id = root.id().to_owned();
    let value = expect_success(root.wait(SETTLE_TIMEOUT)?)?;
    ensure!(value["value"]["callback_result"]["value"] == json!("nested"));
    ensure!(value["pid"] == json!(root.pid()));
    ensure!(broker.trace().iter().any(|event| matches!(
        event,
        TraceEvent::Started { export, parent_id: Some(parent), .. }
            if export.slot == "context" && parent == &root_id
    )));
    Ok(())
}

fn targeted_cancel_preserves_sibling_and_generation() -> Result<()> {
    let broker = broker()?;
    let target = broker.start(
        workflow("spike.workflow"),
        json!({"op":"wait_cancel", "delay_ms":1000}),
    )?;
    let sibling = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"sibling", "delay_ms":80}),
    )?;
    let pid = target.pid();
    let target_id = target.id().to_owned();
    wait_for_trace(&broker, |event| {
        matches!(
            event,
            TraceEvent::Started { id, .. } if id == &target_id
        )
    })?;
    target.cancel(CancelCause::User)?;
    ensure!(target.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::User));
    let sibling_value = expect_success(sibling.wait(SETTLE_TIMEOUT)?)?;
    ensure!(sibling_value["value"] == json!("sibling"));

    let next = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"next"}),
    )?;
    ensure!(next.pid() == pid, "cooperative cancel reset the component");
    ensure!(next.generation() == 1);
    expect_success(next.wait(SETTLE_TIMEOUT)?)?;
    Ok(())
}

fn queued_cancel_never_reaches_worker() -> Result<()> {
    let config = BrokerConfig {
        max_active_roots: 1,
        ..BrokerConfig::default()
    };
    let broker = Broker::spawn(fixture(), config)?;
    let blocker = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"blocker", "delay_ms":120}),
    )?;
    let queued = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"never"}),
    )?;
    let queued_id = queued.id().to_owned();
    queued.cancel(CancelCause::User)?;
    ensure!(queued.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::User));
    ensure_one_terminal(&broker, &queued_id)?;
    expect_success(blocker.wait(SETTLE_TIMEOUT)?)?;
    ensure!(
        broker
            .trace()
            .iter()
            .any(|event| matches!(event, TraceEvent::Queued { id } if id == &queued_id))
    );
    ensure!(
        !broker
            .trace()
            .iter()
            .any(|event| matches!(event, TraceEvent::Started { id, .. } if id == &queued_id))
    );
    Ok(())
}

fn uncooperative_cancel_resets_shared_failure_domain() -> Result<()> {
    let config = BrokerConfig {
        cancel_grace: Duration::from_millis(40),
        ..BrokerConfig::default()
    };
    let broker = Broker::spawn(fixture(), config)?;
    let target = broker.start(
        workflow("spike.workflow"),
        json!({"op":"ignore_cancel", "delay_ms":500}),
    )?;
    let target_id = target.id().to_owned();
    let sibling = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"lost", "delay_ms":500}),
    )?;
    wait_for_trace(
        &broker,
        |event| matches!(event, TraceEvent::Started { id, .. } if id == &target_id),
    )?;
    target.cancel(CancelCause::Timeout)?;
    ensure!(target.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::Timeout));
    ensure!(
        sibling.wait(SETTLE_TIMEOUT)? == Terminal::ComponentLost(ComponentLostCause::CancelGrace)
    );

    let replacement = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"replacement"}),
    )?;
    ensure!(replacement.generation() == 2);
    expect_success(replacement.wait(SETTLE_TIMEOUT)?)?;
    Ok(())
}

fn parent_cancel_cascades_to_nested_invocation() -> Result<()> {
    let broker = broker()?;
    let root = broker.start(
        workflow("spike.workflow"),
        json!({
            "op":"callback",
            "callback_input":{"op":"wait_cancel", "delay_ms":2000}
        }),
    )?;
    let root_id = root.id().to_owned();
    wait_for_trace(&broker, |event| {
        matches!(
            event,
            TraceEvent::Started { parent_id: Some(parent), .. } if parent == &root_id
        )
    })?;
    root.cancel(CancelCause::User)?;
    ensure!(root.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::User));
    ensure_one_terminal(&broker, &root_id)?;
    ensure!(broker.trace().iter().any(|event| matches!(
        event,
        TraceEvent::Terminal { id, outcome: "canceled" } if id != &root_id
    )));
    Ok(())
}

fn callback_parent_and_duplicate_ids_fail_closed() -> Result<()> {
    let hostile_broker = broker()?;
    let forged = hostile_broker.start(
        workflow("spike.workflow"),
        json!({"op":"forged_parent", "forged_parent_id":"h:1:999"}),
    )?;
    ensure!(forged.wait(SETTLE_TIMEOUT)? == Terminal::ComponentLost(ComponentLostCause::Protocol));
    ensure!(hostile_broker.trace().iter().any(|event| matches!(
        event,
        TraceEvent::ProtocolViolation { reason } if reason.contains("forged")
    )));

    let duplicate = hostile_broker.start(
        workflow("spike.workflow"),
        json!({"op":"duplicate_callback_id"}),
    )?;
    ensure!(
        duplicate.wait(SETTLE_TIMEOUT)? == Terminal::ComponentLost(ComponentLostCause::Protocol)
    );
    ensure!(hostile_broker.trace().iter().any(|event| matches!(
        event,
        TraceEvent::ProtocolViolation { reason } if reason.contains("reused")
    )));

    let terminal_parent_broker = broker()?;
    let completed = terminal_parent_broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"done"}),
    )?;
    let terminal_parent_id = completed.id().to_owned();
    expect_success(completed.wait(SETTLE_TIMEOUT)?)?;
    let terminal_parent = terminal_parent_broker.start(
        workflow("spike.workflow"),
        json!({"op":"sibling_parent", "other_active_id":terminal_parent_id}),
    )?;
    ensure!(
        terminal_parent.wait(SETTLE_TIMEOUT)?
            == Terminal::ComponentLost(ComponentLostCause::Protocol)
    );

    let direction_broker = broker()?;
    let wrong_direction = direction_broker.start(
        workflow("spike.workflow"),
        json!({"op":"wrong_callback_direction"}),
    )?;
    ensure!(
        wrong_direction.wait(SETTLE_TIMEOUT)?
            == Terminal::ComponentLost(ComponentLostCause::Protocol)
    );
    ensure!(direction_broker.trace().iter().any(|event| matches!(
        event,
        TraceEvent::ProtocolViolation { reason } if reason.contains("wrong direction")
    )));
    Ok(())
}

fn terminal_with_live_callback_resets_generation() -> Result<()> {
    let broker = broker()?;
    let invocation = broker.start(
        workflow("spike.workflow"),
        json!({
            "op":"terminal_during_callback",
            "callback_input":{"op":"echo", "value":"late", "delay_ms":200}
        }),
    )?;
    ensure!(
        invocation.wait(SETTLE_TIMEOUT)? == Terminal::ComponentLost(ComponentLostCause::Protocol)
    );
    ensure!(broker.trace().iter().any(|event| matches!(
        event,
        TraceEvent::ProtocolViolation { reason } if reason.contains("live callbacks")
    )));
    Ok(())
}

fn late_frame_resets_generation_without_second_terminal() -> Result<()> {
    let broker = broker()?;
    let first = broker.start(
        workflow("spike.workflow"),
        json!({"op":"terminal_then_late", "late_delay_ms":10}),
    )?;
    let first_id = first.id().to_owned();
    expect_success(first.wait(SETTLE_TIMEOUT)?)?;
    wait_for_trace(&broker, |event| {
        matches!(
            event,
            TraceEvent::GenerationReset {
                cause: ComponentLostCause::Protocol,
                ..
            }
        )
    })?;
    ensure!(
        broker
            .trace()
            .iter()
            .filter(|event| matches!(event, TraceEvent::Terminal { id, .. } if id == &first_id))
            .count()
            == 1,
        "late frame produced a second terminal"
    );
    let second = broker.start(workflow("spike.workflow"), json!({"op":"echo", "value":2}))?;
    ensure!(second.generation() == 2);
    expect_success(second.wait(SETTLE_TIMEOUT)?)?;
    Ok(())
}

fn slow_progress_consumer_cannot_block_control_lane() -> Result<()> {
    let config = BrokerConfig {
        notification_capacity: 4,
        ..BrokerConfig::default()
    };
    let broker = Broker::spawn(fixture(), config)?;
    let cancellable = broker.start(
        workflow("spike.workflow"),
        json!({"op":"wait_cancel", "delay_ms":2000}),
    )?;
    let flood = broker.start(
        workflow("spike.workflow"),
        json!({"op":"flood", "count":1000, "payload":"xxxxxxxx"}),
    )?;
    let callback = broker.start(
        workflow("spike.workflow"),
        json!({"op":"callback", "callback_input":{"op":"echo", "value":"responsive"}}),
    )?;
    cancellable.cancel(CancelCause::User)?;
    ensure!(cancellable.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::User));
    let callback_value = expect_success(callback.wait(SETTLE_TIMEOUT)?)?;
    ensure!(callback_value["value"]["callback_result"]["value"] == json!("responsive"));
    expect_success(flood.wait(SETTLE_TIMEOUT)?)?;
    ensure!(flood.drain_notifications().len() <= 4);
    ensure!(
        broker
            .trace()
            .iter()
            .any(|event| matches!(event, TraceEvent::NotificationDropped { .. }))
    );
    Ok(())
}

fn process_exit_is_observed_while_progress_is_buffered() -> Result<()> {
    let broker = broker()?;
    let flood = broker.start(
        workflow("spike.workflow"),
        json!({"op":"flood", "count":300, "payload":"buffered"}),
    )?;
    let exiting = broker.start(
        workflow("spike.workflow"),
        json!({"op":"exit_process", "delay_ms":2}),
    )?;
    ensure!(
        exiting.wait(SETTLE_TIMEOUT)? == Terminal::ComponentLost(ComponentLostCause::ProcessExit)
    );
    let flood_terminal = flood.wait(SETTLE_TIMEOUT)?;
    ensure!(matches!(
        flood_terminal,
        Terminal::Success(_) | Terminal::ComponentLost(ComponentLostCause::ProcessExit)
    ));
    wait_for_trace(&broker, |event| {
        matches!(
            event,
            TraceEvent::GenerationReset {
                cause: ComponentLostCause::ProcessExit,
                ..
            }
        )
    })?;
    Ok(())
}

fn nested_admission_has_depth_count_and_capacity_bounds() -> Result<()> {
    let depth_broker = Broker::spawn(
        fixture(),
        BrokerConfig {
            max_callback_depth: 1,
            ..BrokerConfig::default()
        },
    )?;
    let depth = depth_broker.start(
        workflow("spike.workflow"),
        recursive_callback(2, json!("leaf")),
    )?;
    ensure!(matches!(
        depth.wait(SETTLE_TIMEOUT)?,
        Terminal::ModuleError { .. }
    ));
    ensure!(depth_broker.trace().iter().any(|event| matches!(
        event,
        TraceEvent::CallbackRejected {
            reason: RejectReason::Depth,
            ..
        }
    )));

    let count_broker = Broker::spawn(
        fixture(),
        BrokerConfig {
            max_callback_depth: 4,
            max_callbacks_per_root: 1,
            ..BrokerConfig::default()
        },
    )?;
    let count = count_broker.start(
        workflow("spike.workflow"),
        recursive_callback(2, json!("leaf")),
    )?;
    ensure!(matches!(
        count.wait(SETTLE_TIMEOUT)?,
        Terminal::ModuleError { .. }
    ));
    ensure!(count_broker.trace().iter().any(|event| matches!(
        event,
        TraceEvent::CallbackRejected {
            reason: RejectReason::Count,
            ..
        }
    )));

    let capacity_broker = Broker::spawn(
        fixture(),
        BrokerConfig {
            max_active_nested: 1,
            ..BrokerConfig::default()
        },
    )?;
    let occupying = capacity_broker.start(
        workflow("spike.workflow"),
        json!({"op":"callback", "callback_input":{"op":"wait_cancel", "delay_ms":2000}}),
    )?;
    wait_for_trace(&capacity_broker, |event| {
        matches!(
            event,
            TraceEvent::Started {
                parent_id: Some(_),
                ..
            }
        )
    })?;
    let rejected = capacity_broker.start(
        workflow("spike.workflow"),
        json!({"op":"callback", "callback_input":{"op":"echo", "value":"no slot"}}),
    )?;
    ensure!(matches!(
        rejected.wait(SETTLE_TIMEOUT)?,
        Terminal::ModuleError { .. }
    ));
    ensure!(capacity_broker.trace().iter().any(|event| matches!(
        event,
        TraceEvent::CallbackRejected {
            reason: RejectReason::NestedCapacity,
            ..
        }
    )));
    occupying.cancel(CancelCause::User)?;
    ensure!(occupying.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::User));

    let saturated_broker = Broker::spawn(
        fixture(),
        BrokerConfig {
            max_active_roots: 2,
            max_active_nested: 2,
            ..BrokerConfig::default()
        },
    )?;
    let first = saturated_broker.start(
        workflow("spike.workflow"),
        json!({"op":"callback", "callback_input":{"op":"echo", "value":1, "delay_ms":20}}),
    )?;
    let second = saturated_broker.start(
        workflow("spike.workflow"),
        json!({"op":"callback", "callback_input":{"op":"echo", "value":2, "delay_ms":20}}),
    )?;
    ensure!(
        expect_success(first.wait(SETTLE_TIMEOUT)?)?["value"]["callback_result"]["value"]
            == json!(1)
    );
    ensure!(
        expect_success(second.wait(SETTLE_TIMEOUT)?)?["value"]["callback_result"]["value"]
            == json!(2)
    );
    Ok(())
}

fn queued_deadline_and_root_pending_capacity_are_bounded() -> Result<()> {
    let broker = Broker::spawn(
        fixture(),
        BrokerConfig {
            max_active_roots: 1,
            max_pending_roots: 2,
            ..BrokerConfig::default()
        },
    )?;
    let blocker = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"blocker", "delay_ms":120}),
    )?;
    let queued = broker.start_with_timeout(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"must-not-run"}),
        Duration::from_millis(20),
    )?;
    let queued_id = queued.id().to_owned();
    let overflow = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"overflow"}),
    );
    let overflow_error = match overflow {
        Ok(_) => bail!("third root bypassed the bounded pending map"),
        Err(error) => error,
    };
    ensure!(
        overflow_error
            .to_string()
            .contains("root pending capacity exhausted")
    );
    ensure!(queued.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::Timeout));
    ensure_one_terminal(&broker, &queued_id)?;
    ensure!(
        !broker
            .trace()
            .iter()
            .any(|event| matches!(event, TraceEvent::Started { id, .. } if id == &queued_id))
    );
    expect_success(blocker.wait(SETTLE_TIMEOUT)?)?;
    Ok(())
}

fn receive_notification_trace_and_outbound_bytes_are_bounded() -> Result<()> {
    let receive_broker = Broker::spawn(
        fixture(),
        BrokerConfig {
            receive_limits: proteus_process_host::ReceiveLimits::new(8, 1024 * 1024),
            ..BrokerConfig::default()
        },
    )?;
    let receive_flood = receive_broker.start(
        workflow("spike.workflow"),
        json!({"op":"flood", "count":1000, "payload":"receive-bound"}),
    )?;
    ensure!(
        receive_flood.wait(SETTLE_TIMEOUT)?
            == Terminal::ComponentLost(ComponentLostCause::Resource)
    );

    let retained_broker = Broker::spawn(
        fixture(),
        BrokerConfig {
            notification_capacity: 4,
            max_notification_frame_bytes: 128,
            trace_capacity: 6,
            ..BrokerConfig::default()
        },
    )?;
    let oversized_progress = retained_broker.start(
        workflow("spike.workflow"),
        json!({"op":"flood", "count":20, "payload":"x".repeat(1024)}),
    )?;
    expect_success(oversized_progress.wait(SETTLE_TIMEOUT)?)?;
    ensure!(oversized_progress.drain_notifications().is_empty());
    ensure!(retained_broker.trace().len() <= 6);
    ensure!(
        retained_broker
            .trace()
            .iter()
            .filter(|event| matches!(event, TraceEvent::NotificationDropped { .. }))
            .count()
            == 1
    );

    let outbound_broker = broker()?;
    let oversized_call = outbound_broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"x".repeat(2 * 1024 * 1024)}),
    )?;
    ensure!(
        oversized_call.wait(SETTLE_TIMEOUT)?
            == Terminal::ComponentLost(ComponentLostCause::Resource)
    );

    let writer_broker = Broker::spawn(
        fixture(),
        BrokerConfig {
            max_active_roots: 4,
            max_pending_roots: 4,
            data_queue_capacity: 1,
            ..BrokerConfig::default()
        },
    )?;
    let stopped_reader = writer_broker.start(
        workflow("spike.workflow"),
        json!({"op":"stop_reading", "delay_ms":2000}),
    )?;
    let ready_at = std::time::Instant::now();
    while stopped_reader.drain_notifications().is_empty() {
        ensure!(ready_at.elapsed() < POLL_TIMEOUT);
        thread::sleep(Duration::from_millis(2));
    }
    let first_large = writer_broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"x".repeat(512 * 1024)}),
    )?;
    thread::sleep(Duration::from_millis(10));
    let second_large = writer_broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"y".repeat(512 * 1024)}),
    )?;
    let overflowed_writer = writer_broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"z".repeat(512 * 1024)}),
    )?;
    ensure!(
        overflowed_writer.wait(SETTLE_TIMEOUT)?
            == Terminal::ComponentLost(ComponentLostCause::Resource)
    );
    ensure!(matches!(
        stopped_reader.wait(SETTLE_TIMEOUT)?,
        Terminal::ComponentLost(ComponentLostCause::Resource)
    ));
    ensure!(matches!(
        first_large.wait(SETTLE_TIMEOUT)?,
        Terminal::ComponentLost(ComponentLostCause::Resource)
    ));
    ensure!(matches!(
        second_large.wait(SETTLE_TIMEOUT)?,
        Terminal::ComponentLost(ComponentLostCause::Resource)
    ));
    Ok(())
}

fn blocked_writer_cannot_block_cancel_grace() -> Result<()> {
    let broker = Broker::spawn(
        fixture(),
        BrokerConfig {
            max_active_roots: 3,
            max_pending_roots: 4,
            max_active_total: 6,
            reserved_nested: 3,
            cancel_grace: Duration::from_millis(60),
            control_queue_capacity: 2,
            data_queue_capacity: 2,
            ..BrokerConfig::default()
        },
    )?;
    let blocker = broker.start(
        workflow("spike.workflow"),
        json!({"op":"stop_reading", "delay_ms":2000}),
    )?;
    let started = std::time::Instant::now();
    while blocker.drain_notifications().is_empty() {
        ensure!(
            started.elapsed() < POLL_TIMEOUT,
            "fixture did not stop stdin in time"
        );
        thread::sleep(Duration::from_millis(2));
    }
    let blocked_data = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"x".repeat(512 * 1024)}),
    )?;
    thread::sleep(Duration::from_millis(10));
    let canceled_before_write = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"must-not-be-written"}),
    )?;
    let canceled_before_write_id = canceled_before_write.id().to_owned();
    canceled_before_write.cancel(CancelCause::User)?;
    ensure!(canceled_before_write.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::User));
    ensure!(!broker.trace().iter().any(
        |event| matches!(event, TraceEvent::Started { id, .. } if id == &canceled_before_write_id)
    ));
    let canceled_at = std::time::Instant::now();
    blocker.cancel(CancelCause::User)?;
    ensure!(blocker.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::User));
    ensure!(
        blocked_data.wait(SETTLE_TIMEOUT)?
            == Terminal::ComponentLost(ComponentLostCause::CancelGrace)
    );
    ensure!(
        canceled_at.elapsed() < Duration::from_millis(500),
        "blocked writer stalled broker-owned cancel grace"
    );
    Ok(())
}

fn cancel_command_wins_before_delayed_terminal() -> Result<()> {
    let broker = broker()?;
    for sequence in 0..12 {
        let invocation = broker.start(
            workflow("spike.workflow"),
            json!({"op":"echo", "value":sequence, "delay_ms":15}),
        )?;
        let id = invocation.id().to_owned();
        wait_for_trace(&broker, |event| {
            matches!(
                event,
                TraceEvent::Started { id: started_id, .. } if started_id == &id
            )
        })?;
        invocation.cancel(CancelCause::User)?;
        ensure!(
            invocation.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::User),
            "cancel-vs-terminal winner changed on iteration {sequence}"
        );
        ensure_one_terminal(&broker, &id)?;
    }
    Ok(())
}

fn host_deadline_becomes_targeted_timeout_cancel() -> Result<()> {
    let broker = broker()?;
    let timed = broker.start_with_timeout(
        workflow("spike.workflow"),
        json!({"op":"wait_cancel", "delay_ms":2000}),
        Duration::from_millis(20),
    )?;
    let timed_id = timed.id().to_owned();
    let sibling = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"survived", "delay_ms":60}),
    )?;
    let pid = timed.pid();
    ensure!(timed.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::Timeout));
    ensure_one_terminal(&broker, &timed_id)?;
    ensure!(expect_success(sibling.wait(SETTLE_TIMEOUT)?)?["value"] == json!("survived"));
    let next = broker.start(
        workflow("spike.workflow"),
        json!({"op":"echo", "value":"same"}),
    )?;
    ensure!(
        next.pid() == pid,
        "cooperative deadline reset the worker: {:#?}",
        broker.trace()
    );
    ensure!(
        next.generation() == 1,
        "cooperative deadline reset generation"
    );
    expect_success(next.wait(SETTLE_TIMEOUT)?)?;
    Ok(())
}

fn sibling_parent_is_a_documented_trusted_component_limit() -> Result<()> {
    let broker = broker()?;
    let claimed_parent = broker.start(
        ExportRef::new("context", "authority.owner"),
        json!({"op":"wait_cancel", "delay_ms":2000}),
    )?;
    let claimed_parent_id = claimed_parent.id().to_owned();
    let sibling = broker.start(
        workflow("spike.workflow"),
        json!({
            "op":"sibling_parent",
            "other_active_id":claimed_parent_id,
            "callback_input":{"op":"echo", "value":"accepted-under-sibling"}
        }),
    )?;
    let value = expect_success(sibling.wait(SETTLE_TIMEOUT)?)?;
    ensure!(
        value["value"]["callback_result"]["value"] == json!("accepted-under-sibling"),
        "spike accidentally claims isolation it cannot prove"
    );
    ensure!(broker.trace().iter().any(|event| matches!(
        event,
        TraceEvent::Started { export, parent_id: Some(parent), .. }
            if parent == &claimed_parent_id && export.slot == "search"
    )));
    claimed_parent.cancel(CancelCause::User)?;
    ensure!(claimed_parent.wait(SETTLE_TIMEOUT)? == Terminal::Canceled(CancelCause::User));
    Ok(())
}

fn recursive_callback(depth: usize, leaf: Value) -> Value {
    if depth == 0 {
        return json!({"op":"echo", "value":leaf});
    }
    json!({
        "op":"callback",
        "callback_input":recursive_callback(depth - 1, leaf),
    })
}
