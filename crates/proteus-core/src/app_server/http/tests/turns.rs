use super::*;

#[tokio::test]
async fn route_send_async_returns_run_id_while_domain_turn_keeps_running() {
    let (state, server, _config_dir) = dogfood_loop_state().await;
    let mut event_rx = server.subscribe();
    let run_id = "run-async".to_owned();

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/send-async",
            json!({
                "id": run_id,
                "text": "apply_patch",
                "session_dir": server.session_dir_path(),
            }),
        ),
    )
    .await
    .expect("send-async response");

    assert_eq!(response.status(), StatusCode::OK);
    let output = response_output(response).await;
    let StdioOutput::Response {
        ok: true,
        output: Some(summary),
        ..
    } = output
    else {
        panic!("expected successful send-async response");
    };
    assert_eq!(summary.get("accepted").and_then(Value::as_bool), Some(true));
    assert_eq!(
        summary.get("run_id").and_then(Value::as_str),
        Some(run_id.as_str())
    );
    assert!(summary.get("turn_id").is_none());

    let approval = wait_for_approval_request(&mut event_rx).await;
    assert_eq!(approval.call.name, "apply_patch");
    assert!(state.running_runs.lock().await.contains_key(&run_id));

    let response = route_request(
        state.clone(),
        authed_json_request(
            &session_uri("/cancel", &server),
            json!({
                "id": "cancel-async",
                "target_id": run_id,
            }),
        ),
    )
    .await
    .expect("cancel response");
    assert_eq!(response.status(), StatusCode::OK);
    let output = response_output(response).await;
    assert!(matches!(output, StdioOutput::Response { ok: true, .. }));
    assert!(state.running_runs.lock().await.is_empty());

    server.shutdown().await;
}

#[tokio::test]
async fn route_send_async_queues_second_message_for_same_session() {
    let (state, server, _config_dir) = dogfood_loop_state().await;
    let mut event_rx = server.subscribe();
    let existing_cancellation = CancellationToken::new();
    let existing_receiver = match spawn_send_run(
        &state,
        server.clone(),
        Some("run-existing".to_owned()),
        "apply_patch".to_owned(),
        existing_cancellation.clone(),
    )
    .await
    .expect("spawn existing run")
    {
        SendDispatch::Started(receiver) => receiver,
        SendDispatch::Queued(_) => panic!("first message must start a turn"),
    };
    let _approval = wait_for_approval_request(&mut event_rx).await;

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/send-async",
            json!({
                "id": "run-next",
                "text": "hello",
                "session_dir": server.session_dir_path(),
            }),
        ),
    )
    .await
    .expect("send-async response");

    assert_eq!(response.status(), StatusCode::OK);
    match response_output(response).await {
        StdioOutput::Response {
            id,
            ok,
            output,
            error,
        } => {
            assert_eq!(id.as_deref(), Some("run-next"));
            assert!(ok);
            assert!(error.is_none());
            let output = output.expect("queued receipt");
            assert_eq!(output["accepted"], true);
            assert_eq!(output["queued"], true);
            assert_eq!(output["queued_count"], 1);
            assert_eq!(output["request_id"], "run-next");
            assert!(output.get("run_id").is_none());
            assert!(output.get("turn_id").is_none());
            assert!(output["active_turn_id"].is_string());
        }
        StdioOutput::Event { .. } => panic!("expected command response"),
        _ => panic!("unexpected output variant"),
    }
    assert!(!state.running_runs.lock().await.contains_key("run-next"));
    let pending = server.pending_requests().await;
    assert_eq!(pending.queued_user_messages.len(), 1);
    assert_eq!(pending.queued_user_messages[0].text, "hello");
    let message_id = pending.queued_user_messages[0].message_id;
    let edit = route_request(
        state.clone(),
        authed_json_request(
            "/queue/edit",
            json!({
                "id": "edit-queued", "message_id": message_id, "text": "updated hello",
                "session_dir": server.session_dir_path(),
            }),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(
        response_output(edit).await,
        StdioOutput::Response { ok: true, .. }
    ));
    let updated = server.pending_requests().await;
    assert_eq!(updated.stream_id, pending.stream_id);
    assert!(updated.seq > pending.seq);
    assert_eq!(updated.queued_user_messages[0].message_id, message_id);
    assert_eq!(updated.queued_user_messages[0].text, "updated hello");
    let delete = route_request(state.clone(), authed_json_request("/queue/delete", json!({
        "id": "delete-queued", "message_id": message_id, "session_dir": server.session_dir_path(),
    }))).await.unwrap();
    assert!(matches!(
        response_output(delete).await,
        StdioOutput::Response { ok: true, .. }
    ));
    let removed = server.pending_requests().await;
    assert!(removed.queued_user_messages.is_empty());
    assert!(removed.seq > updated.seq);
    let late = execute_app_request(
        &state,
        StdioRequest::EditQueuedMessage {
            id: None,
            message_id,
            text: "too late".into(),
        },
        Some(&session_query(&server)),
    )
    .await;
    assert!(matches!(late, StdioOutput::Response { ok: false, .. }));

    existing_cancellation.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), existing_receiver).await;
    server.shutdown().await;
}

#[tokio::test]
async fn send_run_cleanup_survives_dropped_waiter() {
    let (state, server, _config_dir) = dogfood_loop_state().await;
    let mut event_rx = server.subscribe();
    let run_id = "sync-send-drop".to_owned();
    let receiver = match spawn_send_run(
        &state,
        server.clone(),
        Some(run_id.clone()),
        "apply_patch".to_owned(),
        CancellationToken::new(),
    )
    .await
    .expect("spawn send run")
    {
        SendDispatch::Started(receiver) => receiver,
        SendDispatch::Queued(_) => panic!("first message must start a turn"),
    };

    let approval = wait_for_approval_request(&mut event_rx).await;
    assert!(state.running_runs.lock().await.contains_key(&run_id));
    drop(receiver);

    let approval_response = route_request(
        state.clone(),
        authed_json_request(
            &session_uri("/approval", &server),
            json!({
                "id": "approval-after-dropped-waiter",
                "approval_id": approval.approval_id,
                "approved": true,
                "note": "finish dropped waiter turn",
                "cache": "none",
            }),
        ),
    )
    .await
    .expect("approval response");
    assert_eq!(approval_response.status(), StatusCode::OK);

    for _ in 0..20 {
        if !state.running_runs.lock().await.contains_key(&run_id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !state.running_runs.lock().await.contains_key(&run_id),
        "send run should unregister itself even when the HTTP waiter is dropped"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn route_send_approval_loop_completes_after_http_approval() {
    let (state, server, _config_dir) = dogfood_loop_state().await;
    let mut event_rx = server.subscribe();
    let session_dir = server.session_dir_path();
    let send_state = state.clone();
    let send_task = tokio::spawn(async move {
        let request = authed_json_request(
            "/send",
            json!({
                "id": "turn-approval",
                "text": "apply_patch",
                "session_dir": session_dir,
            }),
        );
        route_request(send_state, request)
            .await
            .expect("send response")
    });

    let approval = wait_for_approval_request(&mut event_rx).await;
    assert_eq!(approval.call.name, "apply_patch");
    assert_eq!(
        approval.tool_spec.as_ref().map(|spec| spec.name.as_str()),
        Some("apply_patch")
    );
    let preview = approval.preview.as_ref().expect("approval preview");
    assert_eq!(preview.kind, "patch");
    assert_eq!(preview.language.as_deref(), Some("diff"));
    assert!(
        preview
            .body
            .as_deref()
            .is_some_and(|body| body.contains("*** Begin Patch"))
    );

    let approval_response = route_request(
        state.clone(),
        authed_json_request(
            &session_uri("/approval", &server),
            json!({
                "id": "approval-response",
                "approval_id": approval.approval_id,
                "approved": true,
                "note": "approved by route loop test",
                "cache": "exact_call",
            }),
        ),
    )
    .await
    .expect("approval response");
    assert_eq!(approval_response.status(), StatusCode::OK);
    match response_output(approval_response).await {
        StdioOutput::Response { id, ok, error, .. } => {
            assert_eq!(id.as_deref(), Some("approval-response"));
            assert!(ok, "approval response should succeed: {error:?}");
        }
        other => panic!("expected approval response output, got {other:?}"),
    }

    let send_response = tokio::time::timeout(Duration::from_secs(2), send_task)
        .await
        .expect("send should finish after approval")
        .expect("send task should join");
    assert_eq!(send_response.status(), StatusCode::OK);
    match response_output(send_response).await {
        StdioOutput::Response {
            id,
            ok,
            output,
            error,
        } => {
            assert_eq!(id.as_deref(), Some("turn-approval"));
            assert!(ok, "send should succeed after approval: {error:?}");
            let text = output
                .as_ref()
                .and_then(|value| value.get("text"))
                .and_then(Value::as_str)
                .expect("send output text");
            assert!(text.contains("Fake final answer after tool result"));
            assert!(text.contains("patch applier is disabled"));
        }
        other => panic!("expected send response output, got {other:?}"),
    }
    assert!(server.pending_approvals.lock().await.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn route_send_user_input_loop_completes_after_http_response() {
    let (state, server, _config_dir) = dogfood_loop_state().await;
    let mut event_rx = server.subscribe();
    let session_dir = server.session_dir_path();
    let send_state = state.clone();
    let send_task = tokio::spawn(async move {
        let request = authed_json_request(
            "/send",
            json!({
                "id": "turn-input",
                "text": "request_user_input",
                "session_dir": session_dir,
            }),
        );
        route_request(send_state, request)
            .await
            .expect("send response")
    });

    let input = wait_for_user_input_request(&mut event_rx).await;
    assert_eq!(input.questions.len(), 1);
    assert_eq!(
        input.questions[0].question,
        "Which smoke path should continue?"
    );

    let input_response = route_request(
        state.clone(),
        authed_json_request(
            &session_uri("/user-input", &server),
            json!({
                "id": "input-response",
                "request_id": input.request_id,
                "response": {
                    "answers": {
                        "Choice": {
                            "answers": ["Approve"]
                        }
                    }
                }
            }),
        ),
    )
    .await
    .expect("user-input response");
    assert_eq!(input_response.status(), StatusCode::OK);
    match response_output(input_response).await {
        StdioOutput::Response { id, ok, error, .. } => {
            assert_eq!(id.as_deref(), Some("input-response"));
            assert!(ok, "user-input response should succeed: {error:?}");
        }
        other => panic!("expected user-input response output, got {other:?}"),
    }

    let send_response = tokio::time::timeout(Duration::from_secs(2), send_task)
        .await
        .expect("send should finish after user input")
        .expect("send task should join");
    assert_eq!(send_response.status(), StatusCode::OK);
    match response_output(send_response).await {
        StdioOutput::Response {
            id,
            ok,
            output,
            error,
        } => {
            assert_eq!(id.as_deref(), Some("turn-input"));
            assert!(ok, "send should succeed after user input: {error:?}");
            let text = output
                .as_ref()
                .and_then(|value| value.get("text"))
                .and_then(Value::as_str)
                .expect("send output text");
            assert!(text.contains("Fake final answer after tool result"));
            assert!(text.contains("User answered:"));
            assert!(text.contains("Choice: Approve"));
        }
        other => panic!("expected send response output, got {other:?}"),
    }
    assert!(server.pending_user_inputs.lock().await.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn cancel_unknown_turn_returns_protocol_error() {
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

    let output = execute_app_request(
        &state,
        StdioRequest::Cancel {
            id: Some("cancel-1".to_owned()),
            target_id: "missing".to_owned(),
        },
        Some(&session_query(&server)),
    )
    .await;

    match output {
        StdioOutput::Response { ok, error, .. } => {
            assert!(!ok);
            assert_eq!(
                error.as_deref(),
                Some("unknown or completed run id for session: missing")
            );
        }
        StdioOutput::Event { .. } => panic!("expected command response"),
        _ => panic!("unexpected output variant"),
    }
    server.shutdown().await;
}

/// Cancel turn-а больше не деняет pending approvals и user inputs сервера
/// скопом: запись живёт, пока жив её запросивший, и убирается watcher-ом,
/// когда orchestrator дропает свой future.
#[tokio::test]
async fn cancel_active_run_keeps_foreign_pending_requests_until_requester_drops() {
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
    let run_id = "run-cancel".to_owned();
    let cancellation = CancellationToken::new();
    state.running_runs.lock().await.insert(
        run_id.clone(),
        RunningRun::new(cancellation.clone(), server.session_dir_path()),
    );

    let (approval_tx, approval_rx) = tokio::sync::oneshot::channel();
    let approval_id = "approval-cancel".to_owned();
    register_pending_approval(&server, &approval_id, approval_tx).await;

    let (input_tx, input_rx) = tokio::sync::oneshot::channel();
    let request_id = "input-cancel".to_owned();
    register_pending_user_input(&server, &request_id, input_tx).await;

    let output = execute_app_request(
        &state,
        StdioRequest::Cancel {
            id: Some("cancel-1".to_owned()),
            target_id: run_id,
        },
        Some(&session_query(&server)),
    )
    .await;

    match output {
        StdioOutput::Response { ok, error, .. } => {
            assert!(ok, "cancel should succeed: {error:?}");
        }
        StdioOutput::Event { .. } => panic!("expected command response"),
        _ => panic!("unexpected output variant"),
    }

    assert!(cancellation.is_cancelled());
    assert!(state.running_runs.lock().await.is_empty());
    // Записи с живыми запросившими переживают cancel чужого turn-а.
    assert!(server.has_pending_approval(&approval_id).await);
    assert!(server.has_pending_user_input(&request_id).await);

    // Запросившие умирают (например, их turn отменили) -> watcher-ы чистят.
    drop(approval_rx);
    drop(input_rx);
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while server.has_pending_approval(&approval_id).await
        || server.has_pending_user_input(&request_id).await
    {
        assert!(
            std::time::Instant::now() < deadline,
            "orphaned approval and user input should be removed by watchers"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    server.shutdown().await;
}
