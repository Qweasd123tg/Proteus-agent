use super::*;

/// An explicit gate using the same binary/worker pair as the desktop package.
/// Run after prepare.mjs; no model credentials or network provider are involved.
#[test]
#[ignore = "requires packaged backend; run after prepare.mjs with --include-ignored"]
fn packaged_backend_auth_readiness_and_shutdown() {
    let workspace = tempfile::tempdir().unwrap();
    let config = workspace.path().join("desktop-test.toml");
    std::fs::write(
        &config,
        r#"
active_provider = "fake"
[providers.fake]
provider = "fake"
model = "fake-model"
[components.model]
command = "proteus-reference-worker"
[components.model.exports.model.fake]
[module_config.model.fake]
implementation = "fake"
[event_log]
path = ".proteus/events.jsonl"
"#,
    )
    .unwrap();
    let bin_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/bin");
    let backend = Backend::launch(
        &bin_dir,
        workspace.path(),
        config.to_str().unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let origin = backend.connection.app_server_origin.clone();
    assert!(!origin.ends_with(":0"));
    assert!(!backend.diagnostics().contains(&backend.connection.token));
    let bootstrap = backend.request("GET", "/bootstrap").unwrap();
    assert!(bootstrap.starts_with("HTTP/1.1 200 "), "{bootstrap}");
    let body = bootstrap.split("\r\n\r\n").nth(1).unwrap();
    let session_dir = serde_json::from_str::<serde_json::Value>(body).unwrap()["session_dir"]
        .as_str()
        .unwrap()
        .to_owned();
    let encoded_session_dir = percent_encode_query(&session_dir);
    assert!(
        backend
            .request(
                "GET",
                &format!("/history?session_dir={encoded_session_dir}"),
            )
            .unwrap()
            .starts_with("HTTP/1.1 200 ")
    );
    let address: SocketAddr = origin.strip_prefix("http://").unwrap().parse().unwrap();
    let mut unauthorized = TcpStream::connect(address).unwrap();
    unauthorized
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        unauthorized,
        "GET /config?session_dir={encoded_session_dir} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    unauthorized.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 401 "), "{response}");
    // Browser EventSource uses query authentication, and reconnects independently.
    for _ in 0..2 {
        let mut events = TcpStream::connect(address).unwrap();
        events
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(
            events,
            "GET /events?session_dir={encoded_session_dir}&token={} HTTP/1.1\r\nHost: {address}\r\nOrigin: tauri://localhost\r\n\r\n",
            backend.connection.token,
        )
        .unwrap();
        let mut response = String::new();
        let mut bytes = [0; 4096];
        while !response.contains(": connected") {
            let count = events.read(&mut bytes).unwrap();
            assert!(count > 0, "SSE closed before its initial event");
            response.push_str(&String::from_utf8_lossy(&bytes[..count]));
        }
        assert!(response.starts_with("HTTP/1.1 200 "));
        assert!(
            response
                .to_lowercase()
                .contains("content-type: text/event-stream")
        );
    }
    drop(backend);
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(300)).is_err());
    // Cold reopen of the same project must work after the owned process exited.
    drop(
        Backend::launch(
            &bin_dir,
            workspace.path(),
            config.to_str().unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap(),
    );
}

fn percent_encode_query(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[test]
fn readiness_parser_requires_the_explicit_protocol_record() {
    assert!(matches!(
        serde_json::from_str::<ReadyRecord>(
            r#"{"type":"http_ready","origin":"http://127.0.0.1:4242"}"#
        ),
        Ok(ReadyRecord::HttpReady { .. })
    ));
    for line in [
        "Proteus app-server HTTP listening on http://127.0.0.1:4242",
        r#"{"origin":"http://127.0.0.1:4242"}"#,
        r#"{"type":"http_ready","origin":"http://127.0.0.1:4242","token":"unexpected"}"#,
    ] {
        assert!(serde_json::from_str::<ReadyRecord>(line).is_err());
    }
}

#[test]
fn failed_start_returns_an_error() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("proteus");
    std::fs::write(&script, "#!/bin/sh\necho \"$@\" >&2\nexit 7\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let result = Backend::launch(temp.path(), temp.path(), "missing", &AtomicBool::new(false));
    assert!(result.is_err());
    let error = result.err().unwrap().to_string();
    assert!(error.contains("Backend"));
}

#[test]
fn process_diagnostics_redact_the_session_credential() {
    let log = Arc::new(Mutex::new(VecDeque::new()));
    let (send, receive) = mpsc::channel();
    read_output(
        std::io::Cursor::new("failed --token secret-value\n"),
        log.clone(),
        "secret-value".to_owned(),
        Some(send),
    );
    assert!(receive.recv().is_err());
    let log = log.lock().unwrap();
    assert_eq!(log[0], "failed --token [session credential]");
}
