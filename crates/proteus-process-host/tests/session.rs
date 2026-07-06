use std::{
    env,
    io::{self, BufReader, Write},
    time::Duration,
};

use anyhow::{Result, anyhow};
use proteus_process_host::{
    ContentLengthFraming, Framing, NewlineJsonFraming, ProcessHost, ProcessSession, ProcessSpec,
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
        "never_respond" => loop {
            std::thread::sleep(Duration::from_secs(60));
        },
        "exit" => std::process::exit(0),
        other => write_error(framing, writer, id, format!("unknown method: {other}")),
    }
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
