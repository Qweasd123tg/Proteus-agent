use super::*;

#[tokio::test]
async fn route_new_session_registers_an_independently_addressable_session() {
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
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;

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
    assert!(
        state
            .server_for_session_dir(PathBuf::from(next_session_dir).as_path())
            .await
            .is_some()
    );

    let response = route_request(
        state.clone(),
        authed_get_request(&format!("/sessions/current?session_dir={next_session_dir}")),
    )
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

    let response = route_request(
        state.clone(),
        authed_get_request(&format!(
            "/sessions/current?session_dir={original_session_dir}"
        )),
    )
    .await
    .expect("sessions response after resume");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response_bytes(response).await;
    let sessions: Vec<Value> = serde_json::from_slice(&bytes).expect("sessions JSON");
    assert!(
        sessions.iter().any(|session| {
            session.get("session_dir").and_then(Value::as_str) == Some(next_session_dir)
        }),
        "every live empty session should remain listed"
    );

    shutdown_test_servers(&state).await;
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
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;
    let cancellation = CancellationToken::new();
    server
        .register_test_run("run-background", cancellation.clone())
        .await;

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        server
            .running_run_ids()
            .await
            .contains(&"run-background".to_owned())
    );
    let original_session_path = PathBuf::from(&original_session_dir);
    assert!(
        state
            .server_for_session_dir(&original_session_path)
            .await
            .is_some()
    );

    let response = route_request(
        state.clone(),
        authed_get_request(&format!(
            "/sessions/current?session_dir={original_session_dir}"
        )),
    )
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
    shutdown_test_servers(&state).await;
}

#[tokio::test]
async fn route_send_async_targets_requested_session_after_another_is_created() {
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
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;

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
        .expect("new session dir")
        .to_owned();
    let next_server = state
        .server_for_session_dir(PathBuf::from(&next_session_dir).as_path())
        .await
        .expect("new session server");

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
        !next_server
            .transcript()
            .await
            .expect("transcript")
            .iter()
            .any(|message| message.text == "sent to original session")
    );

    shutdown_test_servers(&state).await;
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
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");
    assert_eq!(response.status(), StatusCode::OK);
    let original_cancellation = CancellationToken::new();
    server
        .register_test_run("run-original", original_cancellation.clone())
        .await;

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
    assert!(
        state
            .server_for_session_dir(PathBuf::from(&original_session_dir).as_path())
            .await
            .is_some()
    );

    original_cancellation.cancel();
    shutdown_test_servers(&state).await;
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
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;
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
            &session_uri("/approval", &server),
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

    shutdown_test_servers(&state).await;
}

#[tokio::test]
async fn route_delete_unsaved_live_session_removes_it_without_replacement() {
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
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;

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
    assert_eq!(summary.get("deleted").and_then(Value::as_bool), Some(true));
    assert!(summary.get("active_replaced").is_none());
    assert!(!PathBuf::from(&original_session_dir).exists());
    assert!(
        state
            .server_for_session_dir(PathBuf::from(&original_session_dir).as_path())
            .await
            .is_none()
    );
    assert!(state.all_servers().await.is_empty());
}
