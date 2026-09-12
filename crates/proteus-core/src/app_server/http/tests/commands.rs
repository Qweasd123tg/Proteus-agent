use super::*;

#[tokio::test]
async fn request_dispatch_sets_permission_mode() {
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
        StdioRequest::SetPermissionMode {
            id: Some("mode-1".to_owned()),
            mode: PermissionMode::Auto,
        },
        Some(&session_query(&server)),
    )
    .await;

    match output {
        StdioOutput::Response {
            id,
            ok,
            output,
            error,
        } => {
            assert_eq!(id.as_deref(), Some("mode-1"));
            assert!(ok);
            assert_eq!(
                output
                    .as_ref()
                    .and_then(|value| value.get("mode"))
                    .and_then(Value::as_str),
                Some("auto")
            );
            assert!(error.is_none());
        }
        StdioOutput::Event { .. } => panic!("expected command response"),
    }
    assert_eq!(server.permission_mode().await, PermissionMode::Auto);
    server.shutdown().await;
}

#[tokio::test]
async fn request_dispatch_sets_reasoning_effort() {
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
        StdioRequest::SetReasoningEffort {
            id: Some("effort-1".to_owned()),
            effort: Some("high".to_owned()),
        },
        Some(&session_query(&server)),
    )
    .await;

    match output {
        StdioOutput::Response {
            id,
            ok,
            output,
            error,
        } => {
            assert_eq!(id.as_deref(), Some("effort-1"));
            assert!(ok);
            assert_eq!(
                output
                    .as_ref()
                    .and_then(|value| value.get("effort"))
                    .and_then(Value::as_str),
                Some("high")
            );
            assert!(error.is_none());
        }
        StdioOutput::Event { .. } => panic!("expected command response"),
    }
    let summary = server.config_summary().await;
    assert_eq!(
        summary.pointer("/reasoning/effort").and_then(Value::as_str),
        Some("high")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn reasoning_effort_none_toggles_reasoning() {
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

    // effort «none» выключает рассуждения целиком.
    let output = execute_app_request(
        &state,
        StdioRequest::SetReasoningEffort {
            id: Some("effort-none".to_owned()),
            effort: Some("none".to_owned()),
        },
        Some(&session_query(&server)),
    )
    .await;
    assert!(matches!(output, StdioOutput::Response { ok: true, .. }));
    let summary = server.config_summary().await;
    assert_eq!(
        summary
            .pointer("/reasoning/enabled")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        summary.pointer("/reasoning/effort").and_then(Value::as_str),
        Some("none")
    );

    // Выбор конкретного effort включает рассуждения обратно.
    let output = execute_app_request(
        &state,
        StdioRequest::SetReasoningEffort {
            id: Some("effort-back".to_owned()),
            effort: Some("high".to_owned()),
        },
        Some(&session_query(&server)),
    )
    .await;
    assert!(matches!(output, StdioOutput::Response { ok: true, .. }));
    let summary = server.config_summary().await;
    assert_eq!(
        summary
            .pointer("/reasoning/enabled")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        summary.pointer("/reasoning/effort").and_then(Value::as_str),
        Some("high")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn request_dispatch_sets_model_and_reasoning_enabled() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let mut config = crate::test_model::config();
    config.providers.get_mut("fake").unwrap().reasoning_efforts = vec!["high".into(), "max".into()];
    let server = AgentAppServer::launch(config, cwd.path().to_path_buf(), Some(&config_path))
        .await
        .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;

    let model_output = execute_app_request(
        &state,
        StdioRequest::SetModel {
            id: Some("model-1".to_owned()),
            model: "deepseek-v4-pro".to_owned(),
        },
        Some(&session_query(&server)),
    )
    .await;
    assert!(matches!(
        model_output,
        StdioOutput::Response { ok: true, .. }
    ));

    let reasoning_output = execute_app_request(
        &state,
        StdioRequest::SetReasoningEnabled {
            id: Some("reasoning-1".to_owned()),
            enabled: false,
        },
        Some(&session_query(&server)),
    )
    .await;
    assert!(matches!(
        reasoning_output,
        StdioOutput::Response { ok: true, .. }
    ));

    let summary = server.config_summary().await;
    assert_eq!(
        summary.pointer("/model/name").and_then(Value::as_str),
        Some("deepseek-v4-pro")
    );
    assert_eq!(
        summary
            .pointer("/reasoning/effort_options")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["none", "high", "max"])
    );
    assert_eq!(
        summary
            .pointer("/reasoning/enabled")
            .and_then(Value::as_bool),
        Some(false)
    );
    server.shutdown().await;
}

#[tokio::test]
async fn route_approval_resolves_pending_request_with_auth_and_cors() {
    let (state, server, _config_dir) = test_state().await;
    let (approval_tx, approval_rx) = tokio::sync::oneshot::channel();
    let approval_id = "approval-route".to_owned();
    register_pending_approval(&server, &approval_id, approval_tx).await;
    let request = Request::builder()
        .method(Method::POST)
        .uri(session_uri("/approval", &server))
        .header(ORIGIN, "http://127.0.0.1:1420")
        .header(AUTHORIZATION, "Bearer session-secret")
        .header(CONTENT_TYPE, "application/json")
        .body(json_body(json!({
            "id": "approval-1",
            "approval_id": approval_id,
            "approved": true,
            "note": "route approval",
            "cache": "exact_call",
        })))
        .expect("request");

    let response = route_request(state, request).await.expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("http://127.0.0.1:1420")
    );
    match response_output(response).await {
        StdioOutput::Response { id, ok, error, .. } => {
            assert_eq!(id.as_deref(), Some("approval-1"));
            assert!(ok, "approval response should succeed: {error:?}");
        }
        other => panic!("expected response output, got {other:?}"),
    }

    let approval = approval_rx.await.expect("approval should resolve");
    assert!(approval.approved);
    assert_eq!(approval.note.as_deref(), Some("route approval"));
    assert_eq!(approval.cache, ApprovalCacheScope::ExactCall);
    assert!(server.pending_approvals.lock().await.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn route_user_input_resolves_pending_request_with_auth_and_cors() {
    let (state, server, _config_dir) = test_state().await;
    let (input_tx, input_rx) = tokio::sync::oneshot::channel();
    let request_id = "input-route".to_owned();
    register_pending_user_input(&server, &request_id, input_tx).await;
    let request = Request::builder()
        .method(Method::POST)
        .uri(session_uri("/user-input", &server))
        .header(ORIGIN, "http://127.0.0.1:1420")
        .header(AUTHORIZATION, "Bearer session-secret")
        .header(CONTENT_TYPE, "application/json")
        .body(json_body(json!({
            "id": "input-1",
            "request_id": request_id,
            "response": {
                "answers": {
                    "scope": {
                        "answers": ["small"]
                    }
                }
            }
        })))
        .expect("request");

    let response = route_request(state, request).await.expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("http://127.0.0.1:1420")
    );
    match response_output(response).await {
        StdioOutput::Response { id, ok, error, .. } => {
            assert_eq!(id.as_deref(), Some("input-1"));
            assert!(ok, "user-input response should succeed: {error:?}");
        }
        other => panic!("expected response output, got {other:?}"),
    }

    let response = input_rx.await.expect("user input should resolve");
    assert_eq!(
        response.answers,
        HashMap::from([(
            "scope".to_owned(),
            UserInputAnswer::new(vec!["small".to_owned()])
        )])
    );
    assert!(server.pending_user_inputs.lock().await.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn route_pending_returns_current_pending_requests_with_auth_and_cors() {
    let (state, server, _config_dir) = test_state().await;
    let (approval_tx, _approval_rx) = tokio::sync::oneshot::channel();
    let approval_id = "approval-pending".to_owned();
    register_pending_approval(&server, &approval_id, approval_tx).await;
    let (input_tx, _input_rx) = tokio::sync::oneshot::channel();
    let request_id = "input-pending".to_owned();
    register_pending_user_input(&server, &request_id, input_tx).await;

    let response = route_request(state, authed_get_request(&session_uri("/pending", &server)))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("http://127.0.0.1:1420")
    );
    let bytes = response_bytes(response).await;
    let pending: crate::app_server::AppPendingRequests =
        serde_json::from_slice(&bytes).expect("pending JSON");
    assert_eq!(pending.approvals.len(), 1);
    assert_eq!(pending.approvals[0].approval_id, approval_id);
    assert_eq!(pending.user_inputs.len(), 1);
    assert_eq!(pending.user_inputs[0].request_id, request_id);
    server.shutdown().await;
}
