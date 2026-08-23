use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::{Result, ensure};
use proteus_contracts::contracts::{
    CONTEXT_HOST_SEARCH_METHOD, PROCESS_CONTEXT_BUILD_METHOD, PROCESS_CONTEXT_CONTRACT_VERSION,
    PROCESS_SEARCH_CONTRACT_VERSION, PROCESS_SEARCH_METHOD, PROCESS_WORKFLOW_CONTRACT_VERSION,
    PROCESS_WORKFLOW_METHOD, ProcessComponentExportRef, WORKFLOW_HOST_BUILD_CONTEXT_METHOD,
};
use proteus_module_protocol::v3::{
    AsyncHostRequestDispatcher, CancelCause, ComponentBroker, ComponentBrokerErrorKind,
    ComponentBrokerOptions, ComponentFailure, ComponentHostRequest, HostRequestFuture,
    InvocationHandle, InvocationTerminal, WeakComponentBroker,
};
use proteus_module_protocol::{
    ProcessComponentBinding, ProcessExportBinding, ProcessModuleRpcError,
};
use proteus_process_host::{ProcessSpec, ReceiveLimits};
use serde_json::{Value, json};

const INVOCATION_TIMEOUT: Duration = Duration::from_secs(3);

fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        strict_handshake_and_out_of_order_responses().await?;
        concurrent_exports_and_same_component_reentrancy().await?;
        overlapping_callbacks_keep_parent_authority().await?;
        sibling_parent_is_a_documented_trusted_component_boundary().await?;
        targeted_cancel_keeps_sibling_and_generation().await?;
        deadline_cancel_is_targeted().await?;
        cancel_before_admission_never_starts_queued_work().await?;
        parent_cancel_cascades_during_callback().await?;
        uncooperative_cancel_resets_failure_domain().await?;
        stopped_worker_reader_cannot_block_cancel_grace().await?;
        live_notifications_are_routed_by_invocation().await?;
        notification_overflow_does_not_block_terminal().await?;
        nested_reserve_survives_saturated_roots().await?;
        callback_depth_and_count_are_bounded().await?;
        queued_invocations_are_not_worker_addressable().await?;
        protocol_faults_fail_closed_and_restart_lazily().await?;
        crash_and_resource_fault_fan_out().await?;
        bootstrap_closes_after_runtime_traffic().await?;
        Ok(())
    })
}

async fn concurrent_exports_and_same_component_reentrancy() -> Result<()> {
    let broker = broker(ComponentBrokerOptions::default())?;
    let dispatcher = Arc::new(NestedDispatcher::new(broker.downgrade()));
    let workflow = export("workflow", "fixture.workflow");
    let context = export("context", "fixture.context");
    let mut root = broker
        .start_invocation_with_dispatcher(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({
                "op":"callback",
                "callback_input": {
                    "op":"callback",
                    "callback_input":{"op":"echo", "value":"deep"}
                }
            }),
            INVOCATION_TIMEOUT,
            dispatcher.clone(),
        )
        .await?;
    let mut sibling = broker
        .start_invocation(
            &context,
            PROCESS_CONTEXT_BUILD_METHOD,
            json!({"op":"echo", "value":"context-sibling", "delay_ms":10}),
            INVOCATION_TIMEOUT,
        )
        .await?;

    ensure!(matches!(
        root.result().await?,
        InvocationTerminal::Success(_)
    ));
    ensure!(value(sibling.result().await?)? == json!("context-sibling"));
    ensure!(root.pid() == sibling.pid());
    Ok(())
}

async fn overlapping_callbacks_keep_parent_authority() -> Result<()> {
    let broker = broker(ComponentBrokerOptions::default())?;
    let workflow_seen = Arc::new(Mutex::new(Vec::new()));
    let context_seen = Arc::new(Mutex::new(Vec::new()));
    let workflow_dispatcher = Arc::new(RecordingDispatcher::new(
        "workflow",
        Arc::clone(&workflow_seen),
    ));
    let context_dispatcher = Arc::new(RecordingDispatcher::new(
        "context",
        Arc::clone(&context_seen),
    ));
    let mut workflow = broker
        .start_invocation_with_dispatcher(
            &export("workflow", "fixture.workflow"),
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"callback", "callback_input":{"marker":"w"}}),
            INVOCATION_TIMEOUT,
            workflow_dispatcher,
        )
        .await?;
    let mut context = broker
        .start_invocation_with_dispatcher(
            &export("context", "fixture.context"),
            PROCESS_CONTEXT_BUILD_METHOD,
            json!({"op":"callback", "callback_input":{"marker":"c"}}),
            INVOCATION_TIMEOUT,
            context_dispatcher,
        )
        .await?;
    ensure!(matches!(
        workflow.result().await?,
        InvocationTerminal::Success(_)
    ));
    ensure!(matches!(
        context.result().await?,
        InvocationTerminal::Success(_)
    ));
    ensure!(
        workflow_seen
            .lock()
            .expect("workflow seen mutex")
            .as_slice()
            == [WORKFLOW_HOST_BUILD_CONTEXT_METHOD]
    );
    ensure!(
        context_seen.lock().expect("context seen mutex").as_slice() == [CONTEXT_HOST_SEARCH_METHOD]
    );
    Ok(())
}

async fn sibling_parent_is_a_documented_trusted_component_boundary() -> Result<()> {
    let broker = broker(ComponentBrokerOptions::default())?;
    let owner_seen = Arc::new(Mutex::new(Vec::new()));
    let attacker_seen = Arc::new(Mutex::new(Vec::new()));
    let mut owner = broker
        .start_invocation_with_dispatcher(
            &export("workflow", "fixture.workflow"),
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"owner", "delay_ms":150}),
            INVOCATION_TIMEOUT,
            Arc::new(RecordingDispatcher::new("owner", Arc::clone(&owner_seen))),
        )
        .await?;
    let mut attacker = broker
        .start_invocation_with_dispatcher(
            &export("workflow", "fixture.workflow"),
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"sibling_parent", "other_active_id":owner.id()}),
            INVOCATION_TIMEOUT,
            Arc::new(RecordingDispatcher::new(
                "attacker",
                Arc::clone(&attacker_seen),
            )),
        )
        .await?;

    ensure!(matches!(
        attacker.result().await?,
        InvocationTerminal::Success(_)
    ));
    ensure!(value(owner.result().await?)? == json!("owner"));
    ensure!(owner_seen.lock().expect("owner seen mutex").len() == 1);
    ensure!(
        attacker_seen
            .lock()
            .expect("attacker seen mutex")
            .is_empty()
    );
    Ok(())
}

async fn targeted_cancel_keeps_sibling_and_generation() -> Result<()> {
    let broker = broker(ComponentBrokerOptions::default())?;
    let workflow = export("workflow", "fixture.workflow");
    let mut target = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"wait_cancel", "delay_ms":1500}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    let mut sibling = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"sibling", "delay_ms":80}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    target.cancel(CancelCause::User)?;

    ensure!(target.result().await? == InvocationTerminal::Canceled);
    ensure!(value(sibling.result().await?)? == json!("sibling"));
    let snapshot = broker.snapshot()?;
    ensure!(snapshot.generation == target.generation());
    ensure!(snapshot.pid == Some(target.pid()));
    Ok(())
}

async fn deadline_cancel_is_targeted() -> Result<()> {
    let broker = broker(ComponentBrokerOptions::default())?;
    let workflow = export("workflow", "fixture.workflow");
    let mut target = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"wait_cancel", "delay_ms":1500}),
            Duration::from_millis(50),
        )
        .await?;
    let mut sibling = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"deadline-sibling", "delay_ms":100}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    ensure!(target.result().await? == InvocationTerminal::TimedOut);
    ensure!(value(sibling.result().await?)? == json!("deadline-sibling"));
    ensure!(target.generation() == sibling.generation());
    Ok(())
}

async fn cancel_before_admission_never_starts_queued_work() -> Result<()> {
    let options = ComponentBrokerOptions {
        max_active_roots: 1,
        max_active_total: 2,
        reserved_nested: 1,
        max_active_nested: 1,
        max_callback_depth: 1,
        ..ComponentBrokerOptions::default()
    };
    let broker = broker(options)?;
    let workflow = export("workflow", "fixture.workflow");
    let mut blocker = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"blocker", "delay_ms":100}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    let mut queued = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"must-not-run"}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    queued.cancel(CancelCause::User)?;
    ensure!(queued.result().await? == InvocationTerminal::Canceled);
    ensure!(value(blocker.result().await?)? == json!("blocker"));
    Ok(())
}

async fn parent_cancel_cascades_during_callback() -> Result<()> {
    let broker = broker(ComponentBrokerOptions::default())?;
    let mut root = broker
        .start_invocation_with_dispatcher(
            &export("workflow", "fixture.workflow"),
            PROCESS_WORKFLOW_METHOD,
            json!({
                "op":"callback",
                "callback_input":{"op":"wait_cancel", "delay_ms":1500}
            }),
            INVOCATION_TIMEOUT,
            Arc::new(NestedDispatcher::new(broker.downgrade())),
        )
        .await?;
    wait_for_pending(&broker, 2)?;
    root.cancel(CancelCause::User)?;
    ensure!(root.result().await? == InvocationTerminal::Canceled);
    wait_for_pending(&broker, 0)?;
    Ok(())
}

async fn uncooperative_cancel_resets_failure_domain() -> Result<()> {
    let options = ComponentBrokerOptions {
        cancel_grace: Duration::from_millis(40),
        ..ComponentBrokerOptions::default()
    };
    let broker = broker(options)?;
    let workflow = export("workflow", "fixture.workflow");
    let mut target = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"ignore_cancel", "delay_ms":1000}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    let original_generation = target.generation();
    let original_pid = target.pid();
    let mut sibling = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"lost", "delay_ms":1000}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    target.cancel(CancelCause::Timeout)?;
    ensure!(target.result().await? == InvocationTerminal::TimedOut);
    ensure!(
        sibling.result().await? == InvocationTerminal::ComponentLost(ComponentFailure::CancelGrace)
    );
    let mut replacement = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"replacement"}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    ensure!(replacement.generation() == original_generation + 1);
    ensure!(replacement.pid() != original_pid);
    ensure!(value(replacement.result().await?)? == json!("replacement"));
    Ok(())
}

async fn stopped_worker_reader_cannot_block_cancel_grace() -> Result<()> {
    let options = ComponentBrokerOptions {
        cancel_grace: Duration::from_millis(40),
        ..ComponentBrokerOptions::default()
    };
    let broker = broker(options)?;
    let mut invocation = broker
        .start_invocation(
            &export("workflow", "fixture.workflow"),
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"stop_reading", "delay_ms":1000}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    let generation = invocation.generation();
    let mut notifications = invocation.notifications()?;
    let notification = notifications
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("stop-reading fixture sent no notification"))?;
    ensure!(notification.params["value"]["stdin_stopped"] == json!(true));
    invocation.cancel(CancelCause::User)?;
    ensure!(invocation.result().await? == InvocationTerminal::Canceled);
    ensure!(broker.snapshot()?.generation == generation + 1);
    Ok(())
}

async fn live_notifications_are_routed_by_invocation() -> Result<()> {
    let broker = broker(ComponentBrokerOptions::default())?;
    let workflow = export("workflow", "fixture.workflow");
    let mut left = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"flood", "count":3, "payload":"left"}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    let mut right = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"flood", "count":3, "payload":"right"}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    let mut left_notifications = left.notifications()?;
    let mut right_notifications = right.notifications()?;
    let left_terminal = tokio::spawn(async move { left.result().await });
    let first_left = left_notifications
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("left notification stream ended before terminal"))?;
    ensure!(first_left.params["value"] == "left");
    ensure!(matches!(
        left_terminal.await??,
        InvocationTerminal::Success(_)
    ));
    ensure!(matches!(
        right.result().await?,
        InvocationTerminal::Success(_)
    ));
    for _ in 0..2 {
        let notification = left_notifications
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("left notification stream ended early"))?;
        ensure!(notification.params["value"] == "left");
    }
    for _ in 0..3 {
        let notification = right_notifications
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("right notification stream ended early"))?;
        ensure!(notification.params["value"] == "right");
    }
    Ok(())
}

async fn notification_overflow_does_not_block_terminal() -> Result<()> {
    let options = ComponentBrokerOptions {
        notification_limits: ReceiveLimits::new(2, 512),
        ..ComponentBrokerOptions::default()
    };
    let broker = broker(options)?;
    let mut invocation = broker
        .start_invocation(
            &export("workflow", "fixture.workflow"),
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"flood", "count":100, "payload":"xxxxxxxxxxxxxxxx"}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    ensure!(matches!(
        invocation.result().await?,
        InvocationTerminal::Success(_)
    ));
    ensure!(invocation.dropped_notifications() > 0);
    let mut notifications = invocation.notifications()?;
    let mut retained = 0;
    while notifications.try_recv().is_ok() {
        retained += 1;
    }
    ensure!(retained <= 2);
    Ok(())
}

async fn nested_reserve_survives_saturated_roots() -> Result<()> {
    let options = ComponentBrokerOptions {
        max_active_roots: 2,
        max_active_total: 4,
        reserved_nested: 2,
        max_active_nested: 2,
        max_callback_depth: 2,
        ..ComponentBrokerOptions::default()
    };
    let broker = broker(options)?;
    let dispatcher = Arc::new(NestedDispatcher::new(broker.downgrade()));
    let workflow = export("workflow", "fixture.workflow");
    let mut blocker = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"blocker", "delay_ms":180}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    let mut nested = broker
        .start_invocation_with_dispatcher(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({
                "op":"callback",
                "callback_input":{
                    "op":"callback",
                    "callback_input":{"op":"echo", "value":"nested"}
                }
            }),
            INVOCATION_TIMEOUT,
            dispatcher,
        )
        .await?;
    ensure!(matches!(
        nested.result().await?,
        InvocationTerminal::Success(_)
    ));
    ensure!(value(blocker.result().await?)? == json!("blocker"));
    Ok(())
}

async fn callback_depth_and_count_are_bounded() -> Result<()> {
    let depth_options = ComponentBrokerOptions {
        max_active_roots: 2,
        max_active_total: 3,
        reserved_nested: 1,
        max_active_nested: 1,
        max_callback_depth: 1,
        ..ComponentBrokerOptions::default()
    };
    let depth_broker = broker(depth_options)?;
    let depth_generation = depth_broker.snapshot()?.generation;
    let mut depth = recursive_callback(&depth_broker).await?;
    ensure!(matches!(
        depth.result().await?,
        InvocationTerminal::ModuleError(_)
    ));
    ensure!(depth_broker.snapshot()?.generation == depth_generation);

    let count_options = ComponentBrokerOptions {
        max_callbacks_per_root: 1,
        ..ComponentBrokerOptions::default()
    };
    let count_broker = broker(count_options)?;
    let count_generation = count_broker.snapshot()?.generation;
    let mut count = recursive_callback(&count_broker).await?;
    ensure!(matches!(
        count.result().await?,
        InvocationTerminal::ModuleError(_)
    ));
    ensure!(count_broker.snapshot()?.generation == count_generation);

    let id_options = ComponentBrokerOptions {
        max_callback_ids_per_generation: 1,
        ..ComponentBrokerOptions::default()
    };
    let id_broker = broker(id_options)?;
    let id_generation = id_broker.snapshot()?.generation;
    let mut ids = recursive_callback(&id_broker).await?;
    ensure!(ids.result().await? == InvocationTerminal::ComponentLost(ComponentFailure::Resource));
    ensure!(id_broker.snapshot()?.generation == id_generation + 1);
    Ok(())
}

async fn recursive_callback(broker: &ComponentBroker) -> Result<InvocationHandle> {
    Ok(broker
        .start_invocation_with_dispatcher(
            &export("workflow", "fixture.workflow"),
            PROCESS_WORKFLOW_METHOD,
            json!({
                "op":"callback",
                "callback_input":{
                    "op":"callback",
                    "callback_input":{"op":"echo", "value":"too-deep"}
                }
            }),
            INVOCATION_TIMEOUT,
            Arc::new(NestedDispatcher::new(broker.downgrade())),
        )
        .await?)
}

async fn protocol_faults_fail_closed_and_restart_lazily() -> Result<()> {
    let broker = broker(ComponentBrokerOptions::default())?;
    let workflow = export("workflow", "fixture.workflow");

    let mut terminal_parent = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"terminal-parent"}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    let terminal_parent_id = terminal_parent.id().to_owned();
    ensure!(matches!(
        terminal_parent.result().await?,
        InvocationTerminal::Success(_)
    ));
    let terminal_parent_generation = broker.snapshot()?.generation;
    let mut stale_parent = broker
        .start_invocation_with_dispatcher(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"sibling_parent", "other_active_id":terminal_parent_id}),
            INVOCATION_TIMEOUT,
            Arc::new(RecordingDispatcher::new(
                "stale",
                Arc::new(Mutex::new(Vec::new())),
            )),
        )
        .await?;
    ensure!(
        stale_parent.result().await?
            == InvocationTerminal::ComponentLost(ComponentFailure::Protocol)
    );
    ensure!(broker.snapshot()?.generation == terminal_parent_generation + 1);

    let forbidden_generation = broker.snapshot()?.generation;
    let mut forbidden = broker
        .start_invocation_with_dispatcher(
            &export("search", "fixture.search"),
            PROCESS_SEARCH_METHOD,
            json!({"op":"callback"}),
            INVOCATION_TIMEOUT,
            Arc::new(RecordingDispatcher::new(
                "forbidden",
                Arc::new(Mutex::new(Vec::new())),
            )),
        )
        .await?;
    ensure!(
        forbidden.result().await? == InvocationTerminal::ComponentLost(ComponentFailure::Protocol)
    );
    ensure!(broker.snapshot()?.generation == forbidden_generation + 1);

    let fanout_generation = broker.snapshot()?.generation;
    let mut sibling = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"protocol-lost", "delay_ms":500}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    let mut forged = broker
        .start_invocation_with_dispatcher(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"forged_parent", "forged_parent_id":"h:999:1"}),
            INVOCATION_TIMEOUT,
            Arc::new(RecordingDispatcher::new(
                "forged",
                Arc::new(Mutex::new(Vec::new())),
            )),
        )
        .await?;
    ensure!(
        forged.result().await? == InvocationTerminal::ComponentLost(ComponentFailure::Protocol)
    );
    ensure!(
        sibling.result().await? == InvocationTerminal::ComponentLost(ComponentFailure::Protocol)
    );
    ensure!(broker.snapshot()?.generation == fanout_generation + 1);

    for input in [
        json!({"op":"wrong_callback_direction"}),
        json!({"op":"duplicate_callback_id"}),
        json!({"op":"terminal_during_callback"}),
        json!({"op":"malformed"}),
    ] {
        let before = broker.snapshot()?.generation;
        let mut invocation = broker
            .start_invocation_with_dispatcher(
                &workflow,
                PROCESS_WORKFLOW_METHOD,
                input,
                INVOCATION_TIMEOUT,
                Arc::new(RecordingDispatcher::new(
                    "fault",
                    Arc::new(Mutex::new(Vec::new())),
                )),
            )
            .await?;
        ensure!(
            invocation.result().await?
                == InvocationTerminal::ComponentLost(ComponentFailure::Protocol)
        );
        ensure!(broker.snapshot()?.generation == before + 1);
    }

    let before_late = broker.snapshot()?.generation;
    let mut late = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"terminal_then_late", "late_delay_ms":5}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    ensure!(matches!(
        late.result().await?,
        InvocationTerminal::Success(_)
    ));
    wait_for_generation(&broker, before_late + 1)?;

    let before_duplicate = broker.snapshot()?.generation;
    let mut duplicate = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"duplicate_terminal"}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    ensure!(matches!(
        duplicate.result().await?,
        InvocationTerminal::Success(_)
    ));
    let duplicate_result = duplicate
        .result()
        .await
        .expect_err("terminal must be consumable exactly once");
    ensure!(duplicate_result.kind == ComponentBrokerErrorKind::InvalidInput);
    wait_for_generation(&broker, before_duplicate + 1)?;
    Ok(())
}

async fn queued_invocations_are_not_worker_addressable() -> Result<()> {
    let workflow = export("workflow", "fixture.workflow");
    for operation in [
        "queued_parent_callback",
        "queued_parent_terminal",
        "queued_parent_notification",
    ] {
        let options = ComponentBrokerOptions {
            max_active_roots: 1,
            max_active_total: 2,
            reserved_nested: 1,
            max_active_nested: 1,
            max_callback_depth: 1,
            ..ComponentBrokerOptions::default()
        };
        let broker = broker(options)?;
        let generation = broker.snapshot()?.generation;
        let mut attacker = broker
            .start_invocation_with_dispatcher(
                &workflow,
                PROCESS_WORKFLOW_METHOD,
                json!({"op":operation, "delay_ms":100}),
                INVOCATION_TIMEOUT,
                Arc::new(RecordingDispatcher::new(
                    "queued-attacker",
                    Arc::new(Mutex::new(Vec::new())),
                )),
            )
            .await?;
        let mut queued = broker
            .start_invocation(
                &workflow,
                PROCESS_WORKFLOW_METHOD,
                json!({"op":"echo", "value":"must-remain-unseen"}),
                INVOCATION_TIMEOUT,
            )
            .await?;

        ensure!(
            attacker.result().await?
                == InvocationTerminal::ComponentLost(ComponentFailure::Protocol)
        );
        ensure!(
            queued.result().await? == InvocationTerminal::ComponentLost(ComponentFailure::Protocol)
        );
        ensure!(broker.snapshot()?.generation == generation + 1);
    }
    Ok(())
}

async fn crash_and_resource_fault_fan_out() -> Result<()> {
    let crash_broker = broker(ComponentBrokerOptions::default())?;
    let workflow = export("workflow", "fixture.workflow");
    let mut crash = crash_broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"exit_process", "delay_ms":20}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    let mut sibling = crash_broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"lost", "delay_ms":500}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    ensure!(
        crash.result().await? == InvocationTerminal::ComponentLost(ComponentFailure::ProcessExit)
    );
    ensure!(matches!(
        sibling.result().await?,
        InvocationTerminal::ComponentLost(_)
    ));

    let options = ComponentBrokerOptions {
        receive_limits: ReceiveLimits::new(16, 1024),
        ..ComponentBrokerOptions::default()
    };
    let resource_broker = broker(options)?;
    let mut oversized = resource_broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"oversized", "bytes":4096}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    ensure!(
        oversized.result().await? == InvocationTerminal::ComponentLost(ComponentFailure::Resource)
    );
    Ok(())
}

async fn bootstrap_closes_after_runtime_traffic() -> Result<()> {
    let broker = broker(ComponentBrokerOptions::default())?;
    let search = export("search", "fixture.search");
    let invalid = broker
        .start_invocation(&search, "not.search", json!({}), INVOCATION_TIMEOUT)
        .await
        .expect_err("rejected async traffic must not count as admission");
    ensure!(invalid.kind == ComponentBrokerErrorKind::InvalidInput);
    ensure!(matches!(
        broker.invoke_bootstrap(
            &search,
            PROCESS_SEARCH_METHOD,
            json!({"op":"echo", "value":"bootstrap"}),
            INVOCATION_TIMEOUT,
        )?,
        InvocationTerminal::Success(_)
    ));
    let mut runtime = broker
        .start_invocation(
            &search,
            PROCESS_SEARCH_METHOD,
            json!({"op":"echo", "value":"runtime"}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    ensure!(matches!(
        runtime.result().await?,
        InvocationTerminal::Success(_)
    ));
    let error = broker
        .invoke_bootstrap(
            &search,
            PROCESS_SEARCH_METHOD,
            json!({"op":"echo"}),
            INVOCATION_TIMEOUT,
        )
        .expect_err("bootstrap must close after runtime traffic");
    ensure!(error.kind == ComponentBrokerErrorKind::BootstrapClosed);
    Ok(())
}

async fn strict_handshake_and_out_of_order_responses() -> Result<()> {
    let broker = broker(ComponentBrokerOptions::default())?;
    let workflow = export("workflow", "fixture.workflow");
    let mut slow = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"slow", "delay_ms":80}),
            INVOCATION_TIMEOUT,
        )
        .await?;
    let mut fast = broker
        .start_invocation(
            &workflow,
            PROCESS_WORKFLOW_METHOD,
            json!({"op":"echo", "value":"fast", "delay_ms":5}),
            INVOCATION_TIMEOUT,
        )
        .await?;

    ensure!(value(fast.result().await?)? == json!("fast"));
    ensure!(value(slow.result().await?)? == json!("slow"));
    ensure!(slow.generation() == fast.generation());
    ensure!(slow.pid() == fast.pid());
    Ok(())
}

fn value(terminal: InvocationTerminal) -> Result<Value> {
    let InvocationTerminal::Success(result) = terminal else {
        anyhow::bail!("expected success, got {terminal:?}");
    };
    Ok(result["value"].clone())
}

#[derive(Clone)]
struct NestedDispatcher {
    broker: WeakComponentBroker,
}

impl NestedDispatcher {
    fn new(broker: WeakComponentBroker) -> Self {
        Self { broker }
    }
}

impl AsyncHostRequestDispatcher for NestedDispatcher {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        let dispatcher = self.clone();
        Box::pin(async move {
            let broker = dispatcher.broker.upgrade().ok_or_else(|| {
                ProcessModuleRpcError::new(-32010, "component broker is no longer available")
            })?;
            let (target, method) = match request.method.as_str() {
                WORKFLOW_HOST_BUILD_CONTEXT_METHOD => (
                    export("context", "fixture.context"),
                    PROCESS_CONTEXT_BUILD_METHOD,
                ),
                CONTEXT_HOST_SEARCH_METHOD => {
                    (export("search", "fixture.search"), PROCESS_SEARCH_METHOD)
                }
                method => {
                    return Err(ProcessModuleRpcError::new(
                        -32601,
                        format!("unexpected test callback method {method}"),
                    ));
                }
            };
            let remaining = request.invocation.remaining().min(INVOCATION_TIMEOUT);
            if remaining.is_zero() {
                return Err(ProcessModuleRpcError::new(
                    -32001,
                    "parent invocation deadline expired",
                ));
            }
            let mut nested = broker
                .start_nested_invocation(
                    &request.invocation,
                    &target,
                    method,
                    request.params,
                    remaining,
                    Arc::new(dispatcher.clone()),
                )
                .await
                .map_err(|error| ProcessModuleRpcError::new(-32011, error.to_string()))?;
            match nested
                .result()
                .await
                .map_err(|error| ProcessModuleRpcError::new(-32011, error.to_string()))?
            {
                InvocationTerminal::Success(value) => Ok(value),
                InvocationTerminal::ModuleError(error) => Err(error),
                terminal => Err(ProcessModuleRpcError::new(
                    -32011,
                    format!("nested invocation failed: {terminal:?}"),
                )),
            }
        })
    }
}

#[derive(Clone)]
struct RecordingDispatcher {
    label: &'static str,
    seen: Arc<Mutex<Vec<String>>>,
}

impl RecordingDispatcher {
    fn new(label: &'static str, seen: Arc<Mutex<Vec<String>>>) -> Self {
        Self { label, seen }
    }
}

impl AsyncHostRequestDispatcher for RecordingDispatcher {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        let label = self.label;
        let seen = Arc::clone(&self.seen);
        Box::pin(async move {
            seen.lock()
                .expect("recording dispatcher mutex")
                .push(request.method);
            Ok(json!({"dispatcher":label}))
        })
    }
}

fn wait_for_generation(broker: &ComponentBroker, expected: u64) -> Result<()> {
    for _ in 0..200 {
        if broker.snapshot()?.generation >= expected {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(5));
    }
    anyhow::bail!("component broker did not advance to generation {expected}")
}

fn wait_for_pending(broker: &ComponentBroker, expected: usize) -> Result<()> {
    let mut last = broker.snapshot()?;
    for _ in 0..200 {
        last = broker.snapshot()?;
        if last.pending_invocations == expected {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(5));
    }
    anyhow::bail!(
        "component broker did not reach {expected} pending invocations; last snapshot: {last:?}"
    )
}

fn broker(options: ComponentBrokerOptions) -> Result<ComponentBroker> {
    ComponentBroker::connect(fixture_spec()?, binding()?, options)
}

fn binding() -> Result<ProcessComponentBinding> {
    ProcessComponentBinding::new(
        "fixture-v3",
        [
            ProcessExportBinding::new(
                "workflow",
                "fixture.workflow",
                PROCESS_WORKFLOW_CONTRACT_VERSION,
                json!({}),
            )?,
            ProcessExportBinding::new(
                "context",
                "fixture.context",
                PROCESS_CONTEXT_CONTRACT_VERSION,
                json!({}),
            )?,
            ProcessExportBinding::new(
                "search",
                "fixture.search",
                PROCESS_SEARCH_CONTRACT_VERSION,
                json!({}),
            )?,
        ],
    )
}

fn export(slot: &str, module_id: &str) -> ProcessComponentExportRef {
    ProcessComponentExportRef::new(slot, module_id)
}

fn fixture_spec() -> Result<ProcessSpec> {
    let fixture = fixture_path();
    ensure!(
        fixture.is_file(),
        "fixture {} does not exist",
        fixture.display()
    );
    Ok(ProcessSpec::new("python3").arg(path_argument(&fixture)))
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("multiplex_worker.py")
}

fn path_argument(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
