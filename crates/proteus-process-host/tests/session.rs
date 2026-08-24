use std::{
    collections::BTreeSet,
    env,
    io::{self, BufReader, Write},
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use proteus_process_host::{
    ContentLengthFraming, DEFAULT_ENV_ALLOWLIST, Framing, NewlineJsonFraming, ProcessHost,
    ProcessSession, ProcessSpec, ProcessTransport, ProcessTransportLimits, ReceiveFrameError,
    ReceiveLimits, SendFrameError,
};
use serde_json::{Value, json};

const SHORT_TIMEOUT: Duration = Duration::from_millis(500);

fn main() {
    let result = if env::args().nth(1).as_deref() == Some("__mock_child") {
        run_mock_child()
    } else {
        run_tests()
    };
    if let Err(error) = result {
        eprintln!("{error:?}");
        std::process::exit(1);
    }
}

type NamedTest = (&'static str, fn() -> Result<()>);

fn run_tests() -> Result<()> {
    let tests: &[NamedTest] = &[
        (
            "request_response_newline_framing",
            request_response_newline_framing,
        ),
        (
            "request_response_content_length_framing",
            request_response_content_length_framing,
        ),
        ("raw_frame_round_trip", raw_frame_round_trip),
        (
            "raw_timeout_does_not_terminate_child",
            raw_timeout_does_not_terminate_child,
        ),
        (
            "duplex_transport_serializes_concurrent_frames",
            duplex_transport_serializes_concurrent_frames,
        ),
        (
            "control_frames_overtake_queued_data",
            control_frames_overtake_queued_data,
        ),
        (
            "queued_data_can_be_canceled_before_write",
            queued_data_can_be_canceled_before_write,
        ),
        (
            "writer_frame_and_byte_limits_are_enforced",
            writer_frame_and_byte_limits_are_enforced,
        ),
        (
            "slow_transport_consumer_remains_receive_bounded",
            slow_transport_consumer_remains_receive_bounded,
        ),
        (
            "terminate_wakes_reader_and_exit_waiters",
            terminate_wakes_reader_and_exit_waiters,
        ),
        #[cfg(unix)]
        (
            "terminate_kills_descendants_holding_stdio",
            terminate_kills_descendants_holding_stdio,
        ),
        (
            "host_terminate_interrupts_blocked_request",
            host_terminate_interrupts_blocked_request,
        ),
        (
            "notifications_buffered_during_request_and_drained",
            notifications_buffered_during_request_and_drained,
        ),
        (
            "wait_notification_receives_requested_method",
            wait_notification_receives_requested_method,
        ),
        (
            "timeout_kills_child_and_returns_error",
            timeout_kills_child_and_returns_error,
        ),
        (
            "lazy_restart_after_child_exit",
            lazy_restart_after_child_exit,
        ),
        (
            "explicit_terminate_and_reset_restart_child",
            explicit_terminate_and_reset_restart_child,
        ),
        (
            "initializer_runs_once_per_generation",
            initializer_runs_once_per_generation,
        ),
        (
            "receive_frame_count_bounds_notification_backlog",
            receive_frame_count_bounds_notification_backlog,
        ),
        (
            "receive_aggregate_bytes_bound_notification_backlog",
            receive_aggregate_bytes_bound_notification_backlog,
        ),
        (
            "invalid_receive_limits_fail_before_spawn",
            invalid_receive_limits_fail_before_spawn,
        ),
        (
            "process_spec_clears_unlisted_environment",
            process_spec_clears_unlisted_environment,
        ),
        (
            "process_spec_allowlists_parent_environment",
            process_spec_allowlists_parent_environment,
        ),
        (
            "process_spec_rejects_invalid_environment",
            process_spec_rejects_invalid_environment,
        ),
    ];

    let mut failed = 0usize;
    for (name, test) in tests {
        print!("test {name} ... ");
        io::stdout().flush()?;
        match test() {
            Ok(()) => println!("ok"),
            Err(error) => {
                failed += 1;
                println!("FAILED");
                eprintln!("{name}: {error:?}");
            }
        }
    }
    if failed > 0 {
        return Err(anyhow!("{failed} process-host session test(s) failed"));
    }
    Ok(())
}

fn request_response_newline_framing() -> Result<()> {
    let spec = mock_spec("newline")?;
    let mut session = ProcessSession::spawn(&spec, NewlineJsonFraming::default())?;

    let response = session.request("echo", json!({ "answer": 42 }), SHORT_TIMEOUT)?;

    assert_eq!(response, json!({ "answer": 42 }));
    Ok(())
}

fn request_response_content_length_framing() -> Result<()> {
    let spec = mock_spec("content-length")?;
    let mut session = ProcessSession::spawn(&spec, ContentLengthFraming::default())?;

    let response = session.request("echo", json!({ "answer": 42 }), SHORT_TIMEOUT)?;

    assert_eq!(response, json!({ "answer": 42 }));
    Ok(())
}

fn raw_frame_round_trip() -> Result<()> {
    let spec = mock_spec("newline")?;
    let host = ProcessHost::new(spec, NewlineJsonFraming::default());

    assert_eq!(host.try_recv_frame()?, None);
    host.send_frame(json!({
        "jsonrpc": "2.0",
        "id": "raw-1",
        "method": "echo",
        "params": { "raw": true }
    }))?;
    let response = host.recv_frame(SHORT_TIMEOUT)?;

    assert_eq!(response["id"], "raw-1");
    assert_eq!(response["result"], json!({ "raw": true }));
    Ok(())
}

fn raw_timeout_does_not_terminate_child() -> Result<()> {
    let spec = mock_spec("newline")?;
    let mut session = ProcessSession::spawn(&spec, NewlineJsonFraming::default())?;

    let error = session
        .recv_frame(Duration::from_millis(20))
        .expect_err("raw receive should time out while the child stays idle");
    assert!(
        matches!(error, ReceiveFrameError::Timeout { .. }),
        "unexpected raw receive error: {error}"
    );

    session.send_frame(json!({
        "jsonrpc": "2.0",
        "id": "after-timeout",
        "method": "echo",
        "params": { "alive": true }
    }))?;
    let response = session.recv_frame(SHORT_TIMEOUT)?;
    assert_eq!(response["result"], json!({ "alive": true }));
    Ok(())
}

fn duplex_transport_serializes_concurrent_frames() -> Result<()> {
    const WRITERS: usize = 32;

    let spec = mock_spec("newline")?;
    let mut transport = ProcessTransport::spawn(&spec, NewlineJsonFraming::default())?;
    let barrier = Arc::new(Barrier::new(WRITERS + 1));
    let mut writers = Vec::new();
    for id in 0..WRITERS {
        let writer = transport.frame_writer();
        let barrier = Arc::clone(&barrier);
        writers.push(thread::spawn(move || {
            barrier.wait();
            writer.send_frame(json!({
                "jsonrpc": "2.0",
                "id": format!("concurrent-{id}"),
                "method": "echo",
                "params": { "writer": id }
            }))
        }));
    }

    barrier.wait();
    for writer in writers {
        writer
            .join()
            .map_err(|_| anyhow!("concurrent writer thread panicked"))??;
    }

    let mut observed = BTreeSet::new();
    for _ in 0..WRITERS {
        let response = transport.recv_frame(SHORT_TIMEOUT)?;
        let id = response["id"]
            .as_str()
            .ok_or_else(|| anyhow!("concurrent response id is not a string"))?;
        let writer = response["result"]["writer"]
            .as_u64()
            .ok_or_else(|| anyhow!("concurrent response is missing writer id"))?;
        assert_eq!(id, format!("concurrent-{writer}"));
        observed.insert(writer);
    }
    assert_eq!(observed.len(), WRITERS);
    Ok(())
}

fn control_frames_overtake_queued_data() -> Result<()> {
    let spec = mock_spec("newline")?;
    let limits = ProcessTransportLimits::new(ReceiveLimits::default(), 4).with_control_queue(2);
    let mut transport =
        ProcessTransport::spawn_with_limits(&spec, NewlineJsonFraming::default(), limits)?;
    let writer = transport.frame_writer();
    writer.send_frame(request("pause", "pause_then_echo", json!({"delay_ms":150})))?;
    thread::sleep(Duration::from_millis(20));

    let first = writer.queue_frame(request(
        "data-first",
        "echo",
        json!({"payload":"x".repeat(2 * 1024 * 1024)}),
    ))?;
    wait_for_dispatch_start(&first)?;
    let second = writer.queue_frame(request("data-second", "echo", json!({})))?;
    let control = writer.queue_control_frame(request("control", "echo", json!({})))?;

    assert_eq!(transport.recv_frame(Duration::from_secs(2))?["id"], "pause");
    assert_eq!(
        transport.recv_frame(Duration::from_secs(2))?["id"],
        "data-first"
    );
    assert_eq!(
        transport.recv_frame(Duration::from_secs(2))?["id"],
        "control"
    );
    assert_eq!(
        transport.recv_frame(Duration::from_secs(2))?["id"],
        "data-second"
    );
    first.wait()?;
    control.wait()?;
    second.wait()?;
    Ok(())
}

fn queued_data_can_be_canceled_before_write() -> Result<()> {
    let spec = mock_spec("newline")?;
    let limits = ProcessTransportLimits::new(ReceiveLimits::default(), 4).with_control_queue(2);
    let mut transport =
        ProcessTransport::spawn_with_limits(&spec, NewlineJsonFraming::default(), limits)?;
    let writer = transport.frame_writer();
    writer.send_frame(request(
        "pause-cancel",
        "pause_then_echo",
        json!({"delay_ms":150}),
    ))?;
    thread::sleep(Duration::from_millis(20));

    let blocker = writer.queue_frame(request(
        "blocker",
        "echo",
        json!({"payload":"x".repeat(2 * 1024 * 1024)}),
    ))?;
    wait_for_dispatch_start(&blocker)?;
    let canceled = writer.queue_frame(request("canceled", "echo", json!({})))?;
    assert!(canceled.cancel_before_write());
    let control = writer.queue_control_frame(request("after-cancel", "echo", json!({})))?;

    assert_eq!(
        transport.recv_frame(Duration::from_secs(2))?["id"],
        "pause-cancel"
    );
    assert_eq!(
        transport.recv_frame(Duration::from_secs(2))?["id"],
        "blocker"
    );
    assert_eq!(
        transport.recv_frame(Duration::from_secs(2))?["id"],
        "after-cancel"
    );
    assert!(matches!(
        canceled.wait(),
        Err(SendFrameError::CanceledBeforeWrite)
    ));
    blocker.wait()?;
    control.wait()?;
    assert!(matches!(
        transport.recv_frame(Duration::from_millis(30)),
        Err(ReceiveFrameError::Timeout { .. })
    ));
    Ok(())
}

fn writer_frame_and_byte_limits_are_enforced() -> Result<()> {
    let spec = mock_spec("newline")?;
    let limits = ProcessTransportLimits::new(ReceiveLimits::default(), 4)
        .with_control_queue(2)
        .with_write_byte_limits(3 * 1024 * 1024, 2500 * 1024, 1024 * 1024);
    let mut transport =
        ProcessTransport::spawn_with_limits(&spec, NewlineJsonFraming::default(), limits)?;
    let writer = transport.frame_writer();
    writer.send_frame(request(
        "pause-bounds",
        "pause_then_echo",
        json!({"delay_ms":150}),
    ))?;
    thread::sleep(Duration::from_millis(20));
    let blocker = writer.queue_frame(request(
        "bounded-blocker",
        "echo",
        json!({"payload":"x".repeat(2 * 1024 * 1024)}),
    ))?;
    wait_for_dispatch_start(&blocker)?;

    let bytes_error = writer
        .queue_frame(request(
            "byte-overflow",
            "echo",
            json!({"payload":"x".repeat(1024 * 1024)}),
        ))
        .expect_err("aggregate data writer byte budget must be enforced");
    assert!(matches!(bytes_error, SendFrameError::QueueBytesFull { .. }));
    let frame_error = writer
        .queue_control_frame(request(
            "frame-overflow",
            "echo",
            json!({"payload":"x".repeat(4 * 1024 * 1024)}),
        ))
        .expect_err("per-frame writer limit must be enforced");
    assert!(matches!(frame_error, SendFrameError::FrameTooLarge { .. }));
    transport.terminate()?;
    let _ = blocker.wait();
    Ok(())
}

fn request(id: &str, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
}

fn wait_for_dispatch_start(dispatch: &proteus_process_host::FrameDispatch) -> Result<()> {
    for _ in 0..200 {
        if dispatch.is_started() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(1));
    }
    Err(anyhow!("queued frame never reached the writer"))
}

fn slow_transport_consumer_remains_receive_bounded() -> Result<()> {
    let spec = mock_spec("newline")?;
    let limits = ProcessTransportLimits::new(ReceiveLimits::new(2, 1024 * 1024), 8);
    let mut transport =
        ProcessTransport::spawn_with_limits(&spec, NewlineJsonFraming::default(), limits)?;

    transport.send_frame(json!({
        "jsonrpc": "2.0",
        "id": "slow-consumer",
        "method": "three_notifications_then_echo",
        "params": {}
    }))?;
    thread::sleep(Duration::from_millis(30));

    assert_eq!(transport.recv_frame(SHORT_TIMEOUT)?["method"], "mock/burst");
    assert_eq!(transport.recv_frame(SHORT_TIMEOUT)?["method"], "mock/burst");
    let error = transport
        .recv_frame(SHORT_TIMEOUT)
        .expect_err("a slow consumer must trip the bounded frame queue");
    assert!(
        error
            .to_string()
            .contains("receive buffer exceeded frame count limit")
            || error
                .to_string()
                .contains("receive channel exceeded frame count limit"),
        "unexpected slow-consumer error: {error}"
    );
    Ok(())
}

fn terminate_wakes_reader_and_exit_waiters() -> Result<()> {
    let spec = mock_spec("newline")?;
    let mut transport = ProcessTransport::spawn(&spec, NewlineJsonFraming::default())?;
    let lifecycle = transport.lifecycle();
    let barrier = Arc::new(Barrier::new(4));

    let reader_barrier = Arc::clone(&barrier);
    let reader = thread::spawn(move || {
        reader_barrier.wait();
        let started = Instant::now();
        let result = transport.recv_frame(Duration::from_secs(5));
        (result, started.elapsed())
    });

    let mut waiters = Vec::new();
    for _ in 0..2 {
        let waiter_lifecycle = lifecycle.clone();
        let waiter_barrier = Arc::clone(&barrier);
        waiters.push(thread::spawn(move || {
            waiter_barrier.wait();
            waiter_lifecycle.wait_for_exit(Duration::from_secs(5))
        }));
    }

    barrier.wait();
    thread::sleep(Duration::from_millis(20));
    let first_exit = lifecycle.terminate()?;
    let second_exit = lifecycle.terminate()?;
    assert_eq!(first_exit, second_exit, "terminate must be idempotent");

    let (reader_result, elapsed) = reader
        .join()
        .map_err(|_| anyhow!("transport reader thread panicked"))?;
    assert!(
        matches!(reader_result, Err(ReceiveFrameError::ReaderStopped { .. })),
        "terminate must wake the frame reader: {reader_result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "reader waited for its original timeout: {elapsed:?}"
    );
    for waiter in waiters {
        assert_eq!(
            waiter
                .join()
                .map_err(|_| anyhow!("lifecycle waiter thread panicked"))??,
            Some(first_exit.clone())
        );
    }
    Ok(())
}

#[cfg(unix)]
fn terminate_kills_descendants_holding_stdio() -> Result<()> {
    let spec = ProcessSpec::new("sh").args(["-c", "sleep 5 & wait"]);
    let mut transport = ProcessTransport::spawn(&spec, NewlineJsonFraming::default())?;
    // Let the shell create the background child that inherits the transport
    // pipes. Without process-group termination, join_workers waits for it.
    thread::sleep(Duration::from_millis(50));
    let started = Instant::now();

    transport.terminate()?;

    assert!(
        started.elapsed() < Duration::from_secs(1),
        "process-group termination waited for descendant-held stdio: {:?}",
        started.elapsed()
    );
    Ok(())
}

fn host_terminate_interrupts_blocked_request() -> Result<()> {
    let spec = mock_spec("newline")?;
    let host = Arc::new(ProcessHost::new(spec, NewlineJsonFraming::default()));
    let request_host = Arc::clone(&host);
    let request = thread::spawn(move || {
        let started = Instant::now();
        let result = request_host.request("never_respond", json!({}), Duration::from_secs(5));
        (result, started.elapsed())
    });

    thread::sleep(Duration::from_millis(30));
    host.terminate()?;
    let (result, elapsed) = request
        .join()
        .map_err(|_| anyhow!("blocked host request thread panicked"))?;
    assert!(result.is_err(), "terminated request unexpectedly succeeded");
    assert!(
        elapsed < Duration::from_secs(1),
        "host terminate waited for the request timeout: {elapsed:?}"
    );

    let response = host.request("echo", json!({ "restarted": true }), SHORT_TIMEOUT)?;
    assert_eq!(response, json!({ "restarted": true }));
    Ok(())
}

fn notifications_buffered_during_request_and_drained() -> Result<()> {
    let spec = mock_spec("newline")?;
    let mut session = ProcessSession::spawn(&spec, NewlineJsonFraming::default())?;

    let response = session.request("notify_then_echo", json!({ "ok": true }), SHORT_TIMEOUT)?;
    let notifications = session.drain_notifications();

    assert_eq!(response, json!({ "ok": true }));
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0]["method"], "mock/during_request");
    Ok(())
}

fn wait_notification_receives_requested_method() -> Result<()> {
    let spec = mock_spec("newline")?;
    let mut session = ProcessSession::spawn(&spec, NewlineJsonFraming::default())?;

    session.notify("send_notifications", json!({}))?;
    let notification = session.wait_notification("mock/target", SHORT_TIMEOUT)?;
    let buffered = session.drain_notifications();

    assert_eq!(notification["method"], "mock/target");
    assert_eq!(buffered.len(), 1);
    assert_eq!(buffered[0]["method"], "mock/other");
    Ok(())
}

fn timeout_kills_child_and_returns_error() -> Result<()> {
    let spec = mock_spec("newline")?;
    let mut session = ProcessSession::spawn(&spec, NewlineJsonFraming::default())?;

    let error = session
        .request("never_respond", json!({}), Duration::from_millis(50))
        .expect_err("request should time out");

    assert!(error.to_string().contains("within 50ms"));
    Ok(())
}

fn lazy_restart_after_child_exit() -> Result<()> {
    let spec = mock_spec("newline")?;
    let host = ProcessHost::new(spec, NewlineJsonFraming::default());

    let error = host
        .request("exit", json!({}), SHORT_TIMEOUT)
        .expect_err("first request should fail when child exits");
    assert!(
        error.to_string().contains("closed") || error.to_string().contains("stopped"),
        "unexpected error: {error}"
    );

    let response = host.request("echo", json!({ "after": "restart" }), SHORT_TIMEOUT)?;
    assert_eq!(response, json!({ "after": "restart" }));
    Ok(())
}

fn explicit_terminate_and_reset_restart_child() -> Result<()> {
    let spec = mock_spec("newline")?;
    let host = ProcessHost::new(spec, NewlineJsonFraming::default());

    let first_pid = host.request("pid", json!({}), SHORT_TIMEOUT)?;
    host.terminate()?;
    let second_pid = host.request("pid", json!({}), SHORT_TIMEOUT)?;
    assert_ne!(first_pid, second_pid, "terminate must discard the child");

    host.reset();
    let third_pid = host.request("pid", json!({}), SHORT_TIMEOUT)?;
    assert_ne!(second_pid, third_pid, "reset must discard the child");
    Ok(())
}

fn initializer_runs_once_per_generation() -> Result<()> {
    const CALLERS: usize = 8;

    let spec = mock_spec("newline")?;
    let initialized = Arc::new(AtomicUsize::new(0));
    let initializer_count = Arc::clone(&initialized);
    let host = Arc::new(ProcessHost::with_initializer(
        spec,
        NewlineJsonFraming::default(),
        move |session| {
            initializer_count.fetch_add(1, Ordering::SeqCst);
            session
                .request("echo", json!({ "initialize": true }), SHORT_TIMEOUT)
                .map(|_| ())
        },
    ));

    let barrier = Arc::new(Barrier::new(CALLERS + 1));
    let mut callers = Vec::new();
    for id in 0..CALLERS {
        let host = Arc::clone(&host);
        let barrier = Arc::clone(&barrier);
        callers.push(thread::spawn(move || {
            barrier.wait();
            host.request("echo", json!({ "caller": id }), SHORT_TIMEOUT)
        }));
    }
    barrier.wait();
    for caller in callers {
        caller
            .join()
            .map_err(|_| anyhow!("process host caller thread panicked"))??;
    }
    assert_eq!(initialized.load(Ordering::SeqCst), 1);

    host.terminate()?;
    host.request("echo", json!({ "generation": 2 }), SHORT_TIMEOUT)?;
    assert_eq!(initialized.load(Ordering::SeqCst), 2);
    Ok(())
}

fn receive_frame_count_bounds_notification_backlog() -> Result<()> {
    let spec = mock_spec("newline")?;
    let limits = ReceiveLimits::new(2, 1024 * 1024);
    let mut session =
        ProcessSession::spawn_with_receive_limits(&spec, NewlineJsonFraming::default(), limits)?;

    let error = session
        .request("three_notifications_then_echo", json!({}), SHORT_TIMEOUT)
        .expect_err("third retained notification must exceed the frame budget");

    assert!(
        error
            .to_string()
            .contains("receive buffer exceeded frame count limit"),
        "unexpected count-limit error: {error}"
    );
    assert_eq!(session.drain_notifications().len(), 2);
    Ok(())
}

fn receive_aggregate_bytes_bound_notification_backlog() -> Result<()> {
    let spec = mock_spec("newline")?;
    let first_bytes = serde_json::to_vec(&large_notification(1))?.len();
    let second_bytes = serde_json::to_vec(&large_notification(2))?.len();
    let limits = ReceiveLimits::new(8, first_bytes + second_bytes - 1);
    let mut session =
        ProcessSession::spawn_with_receive_limits(&spec, NewlineJsonFraming::default(), limits)?;

    let error = session
        .request("large_notifications_then_echo", json!({}), SHORT_TIMEOUT)
        .expect_err("second retained notification must exceed aggregate bytes");

    assert!(
        error
            .to_string()
            .contains("receive buffer exceeded aggregate byte limit"),
        "unexpected aggregate-limit error: {error}"
    );
    assert_eq!(session.drain_notifications().len(), 1);
    Ok(())
}

fn invalid_receive_limits_fail_before_spawn() -> Result<()> {
    let spec = ProcessSpec::new("this-command-must-not-be-spawned");
    let error = ProcessSession::spawn_with_receive_limits(
        &spec,
        NewlineJsonFraming::default(),
        ReceiveLimits::new(0, 1024),
    )
    .expect_err("zero frame budget must fail before command spawn");

    assert!(
        error
            .to_string()
            .contains("max_buffered_frames must be greater than zero"),
        "unexpected receive-limit validation error: {error}"
    );

    let error = ProcessTransport::spawn_with_limits(
        &spec,
        NewlineJsonFraming::default(),
        ProcessTransportLimits::new(ReceiveLimits::default(), 0),
    )
    .expect_err("zero writer queue must fail before command spawn");
    assert!(
        error
            .to_string()
            .contains("max_queued_writes must be greater than zero"),
        "unexpected writer-limit validation error: {error}"
    );

    let error = ProcessTransport::spawn_with_limits(
        &spec,
        NewlineJsonFraming::default(),
        ProcessTransportLimits::default().with_control_queue(0),
    )
    .expect_err("zero control writer queue must fail before command spawn");
    assert!(
        error
            .to_string()
            .contains("max_queued_control_writes must be greater than zero"),
        "unexpected control-writer validation error: {error}"
    );

    let error = ProcessTransport::spawn_with_limits(
        &spec,
        NewlineJsonFraming::default(),
        ProcessTransportLimits::default().with_write_byte_limits(0, 1, 1),
    )
    .expect_err("zero outbound frame budget must fail before command spawn");
    assert!(
        error
            .to_string()
            .contains("max_frame_bytes must be greater than zero"),
        "unexpected writer-byte validation error: {error}"
    );
    Ok(())
}

fn process_spec_clears_unlisted_environment() -> Result<()> {
    let (blocked_name, _) = unlisted_parent_environment()?;
    let parent_path = env::var("PATH")?;
    let spec = mock_spec("newline")?.env("PROCESS_HOST_SCOPED", "scoped-value");
    let mut session = ProcessSession::spawn(&spec, NewlineJsonFraming::default())?;

    let values = inspect_environment(
        &mut session,
        &["PATH", blocked_name.as_str(), "PROCESS_HOST_SCOPED"],
    )?;

    assert_eq!(values["PATH"], parent_path);
    assert_eq!(values[&blocked_name], Value::Null);
    assert_eq!(values["PROCESS_HOST_SCOPED"], "scoped-value");
    Ok(())
}

fn process_spec_allowlists_parent_environment() -> Result<()> {
    let (name, parent_value) = unlisted_parent_environment()?;
    let spec = mock_spec("newline")?.env_allowlist([name.clone()]);
    let mut session = ProcessSession::spawn(&spec, NewlineJsonFraming::default())?;

    let values = inspect_environment(&mut session, &[name.as_str()])?;

    assert_eq!(values[&name], parent_value);

    let spec = mock_spec("newline")?
        .env_allowlist([name.clone()])
        .env(name.clone(), "explicit-override");
    let mut session = ProcessSession::spawn(&spec, NewlineJsonFraming::default())?;
    let values = inspect_environment(&mut session, &[name.as_str()])?;
    assert_eq!(values[&name], "explicit-override");
    Ok(())
}

fn process_spec_rejects_invalid_environment() -> Result<()> {
    let spec = ProcessSpec::new("this-command-must-not-be-spawned").env_allowlist(["INVALID=ENV"]);
    let error = ProcessSession::spawn(&spec, NewlineJsonFraming::default())
        .expect_err("invalid env name must fail before command spawn");

    assert!(
        error
            .to_string()
            .contains("invalid environment variable name"),
        "unexpected environment validation error: {error}"
    );

    let spec =
        ProcessSpec::new("this-command-must-not-be-spawned").env("VALID_ENV_NAME", "contains\0nul");
    let error = ProcessSession::spawn(&spec, NewlineJsonFraming::default())
        .expect_err("NUL env value must fail before command spawn");
    assert!(
        error.to_string().contains("contains a NUL byte"),
        "unexpected environment value error: {error}"
    );
    Ok(())
}

fn unlisted_parent_environment() -> Result<(String, String)> {
    env::vars()
        .find(|(name, _)| {
            name != "PROCESS_HOST_SCOPED" && !DEFAULT_ENV_ALLOWLIST.contains(&name.as_str())
        })
        .ok_or_else(|| anyhow!("test process has no environment outside the default allowlist"))
}

fn inspect_environment(
    session: &mut ProcessSession<NewlineJsonFraming>,
    names: &[&str],
) -> Result<Value> {
    session.request(
        "inspect_environment",
        json!({ "names": names }),
        SHORT_TIMEOUT,
    )
}

fn mock_spec(framing: &str) -> Result<ProcessSpec> {
    let exe = env::current_exe()?;
    let exe = exe
        .to_str()
        .ok_or_else(|| anyhow!("test binary path is not UTF-8"))?;
    Ok(ProcessSpec::new(exe).args(["__mock_child", framing]))
}

fn run_mock_child() -> Result<()> {
    match env::args().nth(2).as_deref() {
        Some("newline") => run_mock_child_with(NewlineJsonFraming::default()),
        Some("content-length") => run_mock_child_with(ContentLengthFraming::default()),
        Some(framing) => Err(anyhow!("unknown mock framing: {framing}")),
        None => Err(anyhow!("missing mock framing argument")),
    }
}

fn run_mock_child_with<F: Framing>(framing: F) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    while let Ok(message) = framing.read_frame(&mut reader) {
        if message.get("id").is_some() {
            handle_request(&framing, &mut writer, &message)?;
        } else {
            handle_notification(&framing, &mut writer, &message)?;
        }
    }
    Ok(())
}

fn handle_request<F: Framing, W: Write>(
    framing: &F,
    writer: &mut W,
    message: &Value,
) -> Result<()> {
    let id = message
        .get("id")
        .cloned()
        .ok_or_else(|| anyhow!("request missing id"))?;
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "echo" => write_response(framing, writer, id, message["params"].clone()),
        "pid" => write_response(framing, writer, id, json!(std::process::id())),
        "notify_then_echo" => {
            framing.write_frame(
                writer,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "mock/during_request",
                    "params": { "buffered": true }
                }),
            )?;
            write_response(framing, writer, id, message["params"].clone())
        }
        "three_notifications_then_echo" => {
            for order in 1..=3 {
                framing.write_frame(
                    writer,
                    &json!({
                        "jsonrpc": "2.0",
                        "method": "mock/burst",
                        "params": { "order": order }
                    }),
                )?;
            }
            write_response(framing, writer, id, message["params"].clone())
        }
        "large_notifications_then_echo" => {
            framing.write_frame(writer, &large_notification(1))?;
            framing.write_frame(writer, &large_notification(2))?;
            write_response(framing, writer, id, message["params"].clone())
        }
        "inspect_environment" => {
            let names = message["params"]["names"]
                .as_array()
                .ok_or_else(|| anyhow!("inspect_environment requires names array"))?;
            let mut values = serde_json::Map::new();
            for name in names {
                let name = name
                    .as_str()
                    .ok_or_else(|| anyhow!("environment name must be a string"))?;
                values.insert(
                    name.to_owned(),
                    env::var(name).map(Value::String).unwrap_or(Value::Null),
                );
            }
            write_response(framing, writer, id, Value::Object(values))
        }
        "pause_then_echo" => {
            let delay_ms = message["params"]["delay_ms"].as_u64().unwrap_or(0);
            thread::sleep(Duration::from_millis(delay_ms));
            write_response(framing, writer, id, message["params"].clone())
        }
        "never_respond" => loop {
            std::thread::sleep(Duration::from_secs(60));
        },
        "exit" => std::process::exit(0),
        other => write_error(framing, writer, id, format!("unknown method: {other}")),
    }
}

fn large_notification(order: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "mock/large",
        "params": {
            "order": order,
            "payload": "x".repeat(256)
        }
    })
}

fn handle_notification<F: Framing, W: Write>(
    framing: &F,
    writer: &mut W,
    message: &Value,
) -> Result<()> {
    if message.get("method").and_then(Value::as_str) == Some("send_notifications") {
        framing.write_frame(
            writer,
            &json!({
                "jsonrpc": "2.0",
                "method": "mock/other",
                "params": { "order": 1 }
            }),
        )?;
        framing.write_frame(
            writer,
            &json!({
                "jsonrpc": "2.0",
                "method": "mock/target",
                "params": { "order": 2 }
            }),
        )?;
    }
    Ok(())
}

fn write_response<F: Framing, W: Write>(
    framing: &F,
    writer: &mut W,
    id: Value,
    result: Value,
) -> Result<()> {
    framing.write_frame(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }),
    )
}

fn write_error<F: Framing, W: Write>(
    framing: &F,
    writer: &mut W,
    id: Value,
    message: String,
) -> Result<()> {
    framing.write_frame(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": message },
        }),
    )
}
