use std::{
    env,
    io::{self, BufReader, Write},
    time::Duration,
};

use anyhow::{Result, anyhow};
use proteus_process_host::{
    ContentLengthFraming, Framing, NewlineJsonFraming, ProcessHost, ProcessSession, ProcessSpec,
    ReceiveFrameError, ReceiveLimits,
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
    Ok(())
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
