//! Model-issued terminal calls through real component-v3 tools, journal and replay.
use super::*;
use proteus_contracts::domain::ToolResult;

#[path = "terminal/cancellation.rs"]
mod cancellation;
#[path = "terminal/deadline.rs"]
mod deadline;

#[derive(Clone, Copy)]
enum Mode {
    Pipes,
    Tty,
    Interrupt,
}

fn terminal_config(endpoint: &str) -> AppConfig {
    let mut config = super::config(endpoint, false);
    config.runtime.workflow_timeout_ms = 45000;
    config.tools.enabled = vec!["exec_command".into(), "write_stdin".into()];
    config
}

fn call(id: &str, name: &str, args: Value) -> Value {
    json!({"type": "function_call", "id": format!("item_{id}"), "status": "completed",
        "call_id": id, "name": name, "arguments": args.to_string()})
}

async fn respond(socket: &mut tokio::net::TcpStream, output: Value) {
    let body = json!({"id": "terminal_response", "object": "response", "status": "completed", "output": output}).to_string();
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    socket.shutdown().await.unwrap();
}

async fn accept(listener: &TcpListener) -> (tokio::net::TcpStream, Value) {
    accept_with_timeout(listener, Duration::from_secs(5)).await
}

async fn accept_with_timeout(
    listener: &TcpListener,
    timeout: Duration,
) -> (tokio::net::TcpStream, Value) {
    let (mut socket, _) = tokio::time::timeout(timeout, listener.accept())
        .await
        .unwrap()
        .unwrap();
    let request = super::read_json_request(&mut socket).await;
    (socket, request)
}

fn tool_text<'a>(request: &'a Value, id: &str) -> &'a str {
    request["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output" && item["call_id"] == id)
        .unwrap()["output"]
        .as_str()
        .unwrap()
}

fn session_id(request: &Value) -> i64 {
    tool_text(request, "call_exec")
        .split("Process running with session ID ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

async fn wait_for_file(path: &Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "command did not create {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn serve_terminal(listener: TcpListener, root: PathBuf, mode: Mode) {
    let (mut socket, request) = accept(&listener).await;
    let tools = request["tools"].as_array().unwrap();
    let exec = tools
        .iter()
        .find(|tool| tool["name"] == "exec_command")
        .unwrap();
    assert_eq!(exec["parameters"]["properties"]["tty"]["type"], "boolean");
    let mut args = json!({"yield_time_ms": 250, "with_escalated_permissions": true});
    args["cmd"] = json!(match mode {
        Mode::Pipes =>
            "printf x >> executions; printf first; while [ ! -e release ]; do sleep 0.01; done; printf last-out; printf last-err >&2; exit 7",
        Mode::Tty =>
            "printf x >> executions; printf ready; read line; printf 'reply-%s' \"$line\"; exit 7",
        Mode::Interrupt => "printf x >> executions; sleep 20",
    });
    if matches!(mode, Mode::Tty) {
        args["tty"] = json!(true);
    }
    respond(
        &mut socket,
        json!([call("call_exec", "exec_command", args)]),
    )
    .await;
    let (mut socket, request) = accept(&listener).await;
    let id = session_id(&request);
    wait_for_file(&root.join("executions")).await;
    let first_count = tool_text(&request, "call_exec").matches("first").count();
    let args = match mode {
        Mode::Pipes => json!({"session_id": id, "chars": "not allowed"}),
        Mode::Tty => json!({"session_id": id, "chars": "hello\n", "yield_time_ms": 5000}),
        Mode::Interrupt => json!({"session_id": id, "chars": "\u{3}", "yield_time_ms": 5000}),
    };
    respond(
        &mut socket,
        json!([call("call_input", "write_stdin", args)]),
    )
    .await;
    let (mut socket, request) = accept(&listener).await;
    if matches!(mode, Mode::Pipes) {
        assert!(tool_text(&request, "call_input").contains("stdin is closed for this session"));
        let release = root.join("release");
        let release_task = tokio::spawn(async move {
            // Cross the old process adapter's hard-coded 30-second deadline.
            tokio::time::sleep(Duration::from_secs(31)).await;
            std::fs::write(release, "release").unwrap();
        });
        respond(
            &mut socket,
            json!([call(
                "call_poll",
                "write_stdin",
                json!({"session_id": id, "yield_time_ms": 300000})
            )]),
        )
        .await;
        let (next_socket, next_request) =
            accept_with_timeout(&listener, Duration::from_secs(40)).await;
        release_task.await.unwrap();
        let text = tool_text(&next_request, "call_poll");
        assert!(
            text.contains("last-out") && text.contains("last-err"),
            "{text}"
        );
        assert_eq!(first_count + text.matches("first").count(), 1, "{text}");
        assert!(text.contains("Process exited with code 7"));
        socket = next_socket;
    } else {
        let text = tool_text(&request, "call_input");
        let code = if matches!(mode, Mode::Tty) { 7 } else { 130 };
        assert!(
            text.contains(&format!("Process exited with code {code}")),
            "{text}"
        );
        if matches!(mode, Mode::Tty) {
            assert!(text.contains("reply-hello"));
        }
    }
    respond(
        &mut socket,
        json!([message("final_answer", &["Terminal checked."])]),
    )
    .await;
}

fn history_results(
    history: &[proteus_contracts::model_standard::CanonicalMessage],
) -> Vec<&ToolResult> {
    history
        .iter()
        .flat_map(|message| &message.parts)
        .filter_map(|part| match &part.payload {
            ContentPart::ToolResult { result } => Some(result),
            _ => None,
        })
        .collect()
}

async fn check(mode: Mode) {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = terminal_config(&format!("http://{}", listener.local_addr().unwrap()));
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let server = tokio::spawn(serve_terminal(listener, root.path().to_owned(), mode));
    let runtime = AgentRuntime::builder(config.clone(), root.path().to_owned())
        .with_config_path(Some(&config_path))
        .build_async()
        .await
        .unwrap();
    let output = tokio::time::timeout(
        Duration::from_secs(45),
        runtime.run("Check terminal.".into()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(output.text, "Terminal checked.");
    server.await.unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("executions")).unwrap(),
        "x"
    );
    let session = runtime.session_dir().unwrap().to_owned();
    let projection = SessionStore::open(session.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(projection.history, runtime.history().await);
    let results = history_results(&projection.history);
    let expected_count = if matches!(mode, Mode::Pipes) { 3 } else { 2 };
    assert_eq!(results.len(), expected_count);
    assert!(results[0].ok);
    assert!(results[0].metadata["session_id"].is_i64());
    let terminal = results.last().unwrap();
    assert!(terminal.ok); // Nonzero process exit is tool result data.
    assert_eq!(terminal.metadata["session_id"], Value::Null);
    if matches!(mode, Mode::Pipes) {
        assert!(!results[1].ok);
        assert_eq!(terminal.metadata["yield_time_ms"], 300000);
    }
    assert!(
        projection
            .records
            .iter()
            .any(|record| matches!(&record.entry,
        JournalEntry::TurnSettled(settled) if settled.status == TurnSettlementStatus::Success))
    );
    drop(runtime);
    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        root.path().to_owned(),
        Some(&config_path),
        session.clone(),
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    assert!(
        transcript
            .iter()
            .any(|item| item.text == "Terminal checked.")
    );
    assert!(
        transcript
            .iter()
            .filter_map(|item| item.tool.as_ref())
            .any(|tool| tool.call_id == terminal.call_id && tool.status == "done")
    );
    drop(cold);
    std::fs::remove_file(root.path().join("executions")).unwrap();
    let replay = replay_workflow(
        &session,
        &config,
        &ModuleCatalog::from_config(&config).unwrap(),
        WorkflowReplayOptions::default(),
    )
    .await
    .unwrap();
    assert!(replay.comparison.matched, "{replay:#?}");
    assert!(replay.source_journal_unchanged);
    assert_eq!(replay.tool_calls.replayed, expected_count);
    assert!(!root.path().join("executions").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipe_launch_closed_stdin_and_long_poll_preserve_history_and_replay() {
    check(Mode::Pipes).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_tty_input_and_nonzero_exit_preserve_history_and_replay() {
    check(Mode::Tty).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipe_ctrl_c_is_terminal_data_in_history_and_replay() {
    check(Mode::Interrupt).await;
}
