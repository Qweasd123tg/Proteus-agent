use super::*;

#[tokio::test]
async fn route_new_session_replaces_active_session_dir() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        crate::test_model::config(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .await
    .expect("app server");
    let original_session_dir = server
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("original session dir")
        .to_owned();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");

    assert_eq!(response.status(), StatusCode::OK);
    let output = response_output(response).await;
    let StdioOutput::Response {
        ok: true,
        output: Some(summary),
        ..
    } = output
    else {
        panic!("expected successful new-session response");
    };
    let next_session_dir = summary
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("new session dir");
    assert_ne!(next_session_dir, original_session_dir);
    assert_eq!(
        state
            .current_server()
            .await
            .config_summary()
            .await
            .get("session_dir")
            .and_then(Value::as_str),
        Some(next_session_dir)
    );

    let response = route_request(state.clone(), authed_get_request("/sessions/current"))
        .await
        .expect("sessions response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response_bytes(response).await;
    let sessions: Vec<Value> = serde_json::from_slice(&bytes).expect("sessions JSON");
    assert!(!PathBuf::from(next_session_dir).exists());
    assert!(
        sessions.iter().any(|session| {
            session.get("session_dir").and_then(Value::as_str) == Some(next_session_dir)
                && session.get("message_count").and_then(Value::as_u64) == Some(0)
        }),
        "active empty session should be listed before the first message"
    );

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/resume",
            json!({
                "id": "resume-original",
                "session_dir": original_session_dir,
            }),
        ),
    )
    .await
    .expect("resume response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = route_request(state.clone(), authed_get_request("/sessions/current"))
        .await
        .expect("sessions response after resume");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response_bytes(response).await;
    let sessions: Vec<Value> = serde_json::from_slice(&bytes).expect("sessions JSON");
    assert!(
        !sessions
            .iter()
            .any(|session| session.get("session_dir").and_then(Value::as_str)
                == Some(next_session_dir)),
        "background empty idle session should disappear after switching away"
    );

    server.shutdown().await;
    state.current_server().await.shutdown().await;
}

#[tokio::test]
async fn route_new_session_keeps_background_turn_registered() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        crate::test_model::config(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .await
    .expect("app server");
    let original_session_dir = server
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("original session dir")
        .to_owned();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());
    let cancellation = CancellationToken::new();
    state.running_runs.lock().await.insert(
        "run-background".to_owned(),
        RunningRun::new(
            cancellation.clone(),
            Some(PathBuf::from(&original_session_dir)),
        ),
    );

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state
            .running_runs
            .lock()
            .await
            .contains_key("run-background")
    );
    let original_session_path = PathBuf::from(&original_session_dir);
    assert!(
        state
            .server_for_session_dir(&original_session_path)
            .await
            .is_some()
    );

    let response = route_request(state.clone(), authed_get_request("/sessions/current"))
        .await
        .expect("sessions response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response_bytes(response).await;
    let sessions: Vec<Value> = serde_json::from_slice(&bytes).expect("sessions JSON");
    let background = sessions
        .iter()
        .find(|session| {
            session.get("session_dir").and_then(Value::as_str) == Some(&original_session_dir)
        })
        .expect("running background session should be listed");
    assert_eq!(
        background
            .pointer("/activity/status")
            .and_then(Value::as_str),
        Some("running")
    );
    assert_eq!(
        background
            .pointer("/activity/running_run_ids")
            .and_then(Value::as_array)
            .and_then(|ids| ids.first())
            .and_then(Value::as_str),
        Some("run-background")
    );

    cancellation.cancel();
    server.shutdown().await;
    state.current_server().await.shutdown().await;
}

#[tokio::test]
async fn route_send_async_targets_requested_session_after_current_switches() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        crate::test_model::config(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .await
    .expect("app server");
    let original_session_dir = server
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("original session dir")
        .to_owned();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");
    assert_eq!(response.status(), StatusCode::OK);
    let current_after_switch = state.current_server().await;
    assert!(!current_after_switch.is_session_dir(PathBuf::from(&original_session_dir).as_path()));

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/send-async",
            json!({
                "id": "send-old-session",
                "text": "sent to original session",
                "session_dir": original_session_dir,
            }),
        ),
    )
    .await
    .expect("send response");
    assert_eq!(response.status(), StatusCode::OK);
    match response_output(response).await {
        StdioOutput::Response { ok: true, .. } => {}
        other => panic!("expected successful send response, got {other:?}"),
    }

    let original_transcript = wait_for_transcript_text(&server, "sent to original session").await;
    assert!(
        original_transcript
            .iter()
            .any(|message| message.role == "user" && message.text == "sent to original session")
    );
    assert!(
        !state
            .current_server()
            .await
            .transcript()
            .await
            .expect("transcript")
            .iter()
            .any(|message| message.text == "sent to original session")
    );

    server.shutdown().await;
    state.current_server().await.shutdown().await;
}

#[tokio::test]
async fn route_resume_reuses_live_session_without_persisted_directory() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        crate::test_model::config(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .await
    .expect("app server");
    let original_session_dir = server
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("original session dir")
        .to_owned();
    assert!(!PathBuf::from(&original_session_dir).exists());
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");
    assert_eq!(response.status(), StatusCode::OK);
    let original_cancellation = CancellationToken::new();
    state.running_runs.lock().await.insert(
        "run-original".to_owned(),
        RunningRun::new(
            original_cancellation.clone(),
            Some(PathBuf::from(&original_session_dir)),
        ),
    );

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/resume",
            json!({
                "id": "resume-original",
                "session_dir": original_session_dir.clone(),
            }),
        ),
    )
    .await
    .expect("resume response");

    assert_eq!(response.status(), StatusCode::OK);
    let output = response_output(response).await;
    let StdioOutput::Response {
        ok: true,
        output: Some(summary),
        ..
    } = output
    else {
        panic!("expected successful resume response");
    };
    assert_eq!(
        summary.get("session_dir").and_then(Value::as_str),
        Some(original_session_dir.as_str())
    );
    assert_eq!(
        summary.pointer("/activity/status").and_then(Value::as_str),
        Some("running")
    );
    assert_eq!(
        summary
            .pointer("/activity/running_run_ids")
            .and_then(Value::as_array)
            .and_then(|ids| ids.first())
            .and_then(Value::as_str),
        Some("run-original")
    );
    assert_eq!(
        state
            .current_server()
            .await
            .config_summary()
            .await
            .get("session_dir")
            .and_then(Value::as_str),
        Some(original_session_dir.as_str())
    );

    original_cancellation.cancel();
    state.current_server().await.shutdown().await;
}

#[tokio::test]
async fn route_approval_resolves_background_session_request() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        crate::test_model::config(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .await
    .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());
    let (responder, response_rx) = tokio::sync::oneshot::channel();
    let approval_id = "approval-background".to_owned();
    register_pending_approval(&server, &approval_id, responder).await;

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/approval",
            json!({
                "id": "approval-response",
                "approval_id": approval_id,
                "approved": true,
                "note": "approved in background",
                "cache": "none",
            }),
        ),
    )
    .await
    .expect("approval response");

    assert_eq!(response.status(), StatusCode::OK);
    match response_output(response).await {
        StdioOutput::Response { ok, error, .. } => {
            assert!(ok, "approval response should succeed: {error:?}");
        }
        other => panic!("expected response output, got {other:?}"),
    }
    let approval = response_rx.await.expect("approval should resolve");
    assert!(approval.approved);
    assert_eq!(approval.note.as_deref(), Some("approved in background"));

    server.shutdown().await;
    state.current_server().await.shutdown().await;
}

#[tokio::test]
async fn route_delete_unsaved_active_session_opens_new_one() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        crate::test_model::config(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .await
    .expect("app server");
    let original_session_dir = server
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("original session dir")
        .to_owned();
    server.start_session().await.expect("start session");
    assert!(!PathBuf::from(&original_session_dir).exists());
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/delete-session",
            json!({
                "id": "delete-session",
                "session_dir": original_session_dir,
            }),
        ),
    )
    .await
    .expect("delete session response");

    assert_eq!(response.status(), StatusCode::OK);
    let output = response_output(response).await;
    let StdioOutput::Response {
        ok: true,
        output: Some(summary),
        ..
    } = output
    else {
        panic!("expected successful delete-session response");
    };
    assert_eq!(summary.get("deleted").and_then(Value::as_bool), Some(false));
    assert_eq!(
        summary.get("active_replaced").and_then(Value::as_bool),
        Some(true)
    );
    assert!(!PathBuf::from(&original_session_dir).exists());
    let next_session_dir = state
        .current_server()
        .await
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("next session dir")
        .to_owned();
    assert_ne!(next_session_dir, original_session_dir);
    assert!(!PathBuf::from(next_session_dir).exists());

    server.shutdown().await;
    state.current_server().await.shutdown().await;
}
