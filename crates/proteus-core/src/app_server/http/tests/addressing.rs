use super::*;

fn scoped_path(path: &str, server: &AppServerHandle) -> String {
    // Test temp paths contain only URI-safe ASCII bytes.
    format!(
        "{path}?session_dir={}",
        server
            .session_dir_path()
            .expect("session storage")
            .display()
    )
}

async fn read_json_at(state: &HttpAppState, path: &str) -> Value {
    let response = route_request(state.clone(), authed_get_request(path))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    serde_json::from_slice(&response_bytes(response).await).unwrap()
}

async fn post_at(state: &HttpAppState, path: &str, body: Value) -> StdioOutput {
    let response = route_request(state.clone(), authed_json_request(path, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    response_output(response).await
}

fn command_value(output: StdioOutput) -> Value {
    match output {
        StdioOutput::Response {
            ok: true,
            output: Some(value),
            ..
        } => value,
        other => panic!("expected command result, got {other:?}"),
    }
}

async fn next_message(body: &mut HttpBody) -> String {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let frame = body.frame().await.expect("SSE open").expect("SSE frame");
            let Ok(data) = frame.into_data() else {
                continue;
            };
            for line in std::str::from_utf8(&data).unwrap().lines() {
                let Some(json) = line.strip_prefix("data: ") else {
                    continue;
                };
                let output: StdioOutput = serde_json::from_str(json).unwrap();
                if let StdioOutput::Event { event } = output {
                    if let AppServerEvent::UserMessageSubmitted { text } = *event {
                        return text;
                    }
                }
            }
        }
    })
    .await
    .expect("addressed message arrives")
}

#[tokio::test]
async fn independent_connections_keep_config_pending_and_sse_bound_to_their_sessions() {
    let cwd = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let a = AgentAppServer::launch(
        crate::test_model::config(),
        cwd.path().to_owned(),
        Some(&config_path),
    )
    .await
    .unwrap();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(a.clone(), shutdown, test_security()).await;
    let initial = read_json_at(&state, "/bootstrap").await;
    let _: proteus_contracts::app_protocol::AppBootstrap =
        serde_json::from_value(initial.clone()).unwrap();
    let created = command_value(post_at(&state, "/new-session", json!({"id":"create-b"})).await);
    let b_dir = PathBuf::from(created["session_dir"].as_str().unwrap());
    let b = state.server_for_session_dir(&b_dir).await.unwrap();

    command_value(
        post_at(
            &state,
            "/mode",
            json!({"mode":"plan", "session_dir":a.session_dir_path()}),
        )
        .await,
    );
    command_value(
        post_at(
            &state,
            "/mode",
            json!({"mode":"auto", "session_dir":b.session_dir_path()}),
        )
        .await,
    );
    assert_eq!(
        read_json_at(&state, &scoped_path("/config", &a)).await["permission_mode"],
        "Plan"
    );
    assert_eq!(
        read_json_at(&state, &scoped_path("/config", &b)).await["permission_mode"],
        "Auto"
    );

    let mut events_a = route_request(
        state.clone(),
        authed_get_request(&scoped_path("/events", &a)),
    )
    .await
    .unwrap()
    .into_body();
    let mut events_b = route_request(
        state.clone(),
        authed_get_request(&scoped_path("/events", &b)),
    )
    .await
    .unwrap()
    .into_body();
    // Start both independent HTTP responses before publishing any session events.
    events_a.frame().await.unwrap().unwrap();
    events_b.frame().await.unwrap().unwrap();
    let (approval_tx_a, approval_rx_a) = tokio::sync::oneshot::channel();
    let (approval_tx_b, _approval_rx_b) = tokio::sync::oneshot::channel();
    register_pending_approval(&a, "approval-a", approval_tx_a).await;
    register_pending_approval(&b, "approval-b", approval_tx_b).await;
    assert_eq!(
        read_json_at(&state, &scoped_path("/pending", &a)).await["approvals"][0]["approval_id"],
        "approval-a"
    );
    assert_eq!(
        read_json_at(&state, &scoped_path("/pending", &b)).await["approvals"][0]["approval_id"],
        "approval-b"
    );

    // Opening B and then A never alters another connection or the bootstrap suggestion.
    for server in [&b, &a] {
        command_value(
            post_at(
                &state,
                "/resume",
                json!({"session_dir":server.session_dir_path()}),
            )
            .await,
        );
    }
    assert_eq!(read_json_at(&state, "/bootstrap").await, initial);
    assert_eq!(state.all_servers().await.len(), 2);
    b.events
        .send(AppServerEvent::UserMessageSubmitted {
            text: "B only".into(),
        })
        .unwrap();
    a.events
        .send(AppServerEvent::UserMessageSubmitted {
            text: "A only".into(),
        })
        .unwrap();
    assert_eq!(next_message(&mut events_a).await, "A only");
    assert_eq!(next_message(&mut events_b).await, "B only");

    // Reconnect B while A was most recently resumed. The URL retains B.
    drop(events_b);
    let mut events_b = route_request(
        state.clone(),
        authed_get_request(&scoped_path("/events", &b)),
    )
    .await
    .unwrap()
    .into_body();
    events_b.frame().await.unwrap().unwrap();
    a.events
        .send(AppServerEvent::UserMessageSubmitted {
            text: "A second".into(),
        })
        .unwrap();
    b.events
        .send(AppServerEvent::UserMessageSubmitted {
            text: "B reconnected".into(),
        })
        .unwrap();
    assert_eq!(next_message(&mut events_b).await, "B reconnected");
    assert_eq!(next_message(&mut events_a).await, "A second");

    let answer = json!({"approval_id":"approval-a", "approved":true});
    assert!(matches!(
        post_at(&state, &scoped_path("/approval", &b), answer.clone()).await,
        StdioOutput::Response { ok: false, .. }
    ));
    assert!(a.has_pending_approval("approval-a").await);
    assert!(matches!(
        post_at(&state, &scoped_path("/approval", &a), answer).await,
        StdioOutput::Response { ok: true, .. }
    ));
    assert!(approval_rx_a.await.unwrap().approved);
    assert!(b.has_pending_approval("approval-b").await);

    let cancel_a = CancellationToken::new();
    let cancel_b = CancellationToken::new();
    state.running_runs.lock().await.insert(
        "run-a".into(),
        RunningRun::new(cancel_a.clone(), a.session_dir_path()),
    );
    state.running_runs.lock().await.insert(
        "run-b".into(),
        RunningRun::new(cancel_b.clone(), b.session_dir_path()),
    );
    assert!(matches!(
        post_at(
            &state,
            &scoped_path("/cancel", &b),
            json!({"target_id":"run-a"})
        )
        .await,
        StdioOutput::Response { ok: false, .. }
    ));
    assert!(!cancel_a.is_cancelled());
    assert!(matches!(
        post_at(
            &state,
            &scoped_path("/cancel", &a),
            json!({"target_id":"run-a"})
        )
        .await,
        StdioOutput::Response { ok: true, .. }
    ));
    assert!(cancel_a.is_cancelled());
    assert!(!cancel_b.is_cancelled());

    drop(events_a);
    drop(events_b);
    command_value(
        post_at(
            &state,
            "/delete-session",
            json!({"session_dir":a.session_dir_path()}),
        )
        .await,
    );
    assert_eq!(
        read_json_at(&state, "/bootstrap").await["session_dir"],
        Value::Null
    );
    assert!(state.server_for_session_dir(&b_dir).await.is_some());
    assert_eq!(
        state.all_servers().await.len(),
        1,
        "deletion does not create a replacement chat"
    );
    assert_eq!(
        read_json_at(&state, &scoped_path("/config", &b)).await["permission_mode"],
        "Auto"
    );
    b.shutdown().await;
}

#[tokio::test]
async fn session_routes_reject_missing_unknown_and_ambiguous_addresses() {
    let cwd = tempfile::tempdir().unwrap();
    let config_path = cwd.path().join("config.toml");
    let server = AgentAppServer::launch(
        crate::test_model::config(),
        cwd.path().to_owned(),
        Some(&config_path),
    )
    .await
    .unwrap();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;
    for path in [
        "/events",
        "/config",
        "/pending",
        "/model/quota",
        "/history",
        "/usage",
        "/context",
        "/sessions/current",
        "/config/builder",
        "/inspect/topology",
        "/inspect/plan",
    ] {
        for suffix in [
            "",
            "?session_dir=",
            "?session_dir=relative",
            "?session_dir=/missing-session",
            "?session_dir=/one&session_dir=/two",
        ] {
            let response = route_request(
                state.clone(),
                authed_get_request(&format!("{path}{suffix}")),
            )
            .await
            .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}{suffix}");
        }
    }
    for (path, body) in [
        ("/send-async", json!({"text":"must not run"})),
        ("/send", json!({"text":"must not run"})),
        ("/mode", json!({"mode":"auto"})),
        ("/model", json!({"model":"wrong"})),
        ("/effort", json!({"effort":"high"})),
        ("/reasoning", json!({"enabled":false})),
        ("/config/web", json!({"tool_cards_collapsed":false})),
        ("/config/builder", json!({"modules":{}})),
    ] {
        let response = route_request(state.clone(), authed_json_request(path, body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
    }
    for body in [
        json!({"type":"config_summary"}),
        json!({"type":"send","text":"must not run"}),
    ] {
        assert!(matches!(
            post_at(&state, "/request", body).await,
            StdioOutput::Response { ok: false, .. }
        ));
    }
    assert_eq!(server.permission_mode().await, PermissionMode::Normal);
    assert!(server.transcript().await.unwrap().is_empty());
    assert!(state.running_runs.lock().await.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn lifecycle_uses_the_explicit_source_workspace_and_keeps_deletion_in_the_config_store() {
    let workspace_a = tempfile::tempdir().unwrap();
    let workspace_b = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let a = AgentAppServer::launch(
        crate::test_model::config(),
        workspace_a.path().to_owned(),
        Some(&config_path),
    )
    .await
    .unwrap();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(a.clone(), shutdown, test_security()).await;
    let stored_b = crate::core::SessionStore::new(
        config_dir.path(),
        workspace_b.path(),
        crate::domain::new_session_id(),
    )
    .unwrap();
    stored_b
        .append_history(
            crate::domain::new_thread_id(),
            None,
            &[crate::model_standard::CanonicalMessage::text(
                crate::model_standard::MessageRole::User,
                "workspace B history",
            )],
        )
        .await
        .unwrap();
    command_value(
        post_at(
            &state,
            "/resume",
            json!({"session_dir":stored_b.session_dir()}),
        )
        .await,
    );
    command_value(
        post_at(
            &state,
            "/mode",
            json!({"session_dir":stored_b.session_dir(),"mode":"auto"}),
        )
        .await,
    );
    let created = command_value(
        post_at(
            &state,
            "/new-session",
            json!({"source_session_dir":stored_b.session_dir()}),
        )
        .await,
    );
    assert_eq!(created["cwd"], workspace_b.path().to_str().unwrap());
    assert_eq!(created["permission_mode"], "Auto");
    let created_dir = PathBuf::from(created["session_dir"].as_str().unwrap());
    assert!(created_dir.starts_with(config_dir.path()));
    assert!(
        command_value(
            post_at(
                &state,
                "/delete-session",
                json!({"session_dir":stored_b.session_dir()})
            )
            .await
        )["deleted"]
            .as_bool()
            .unwrap()
    );
    assert!(!stored_b.session_dir().exists());
    assert_eq!(
        read_json_at(&state, "/bootstrap").await["session_dir"],
        a.session_dir_path().unwrap().to_str().unwrap()
    );
    assert!(state.server_for_session_dir(&created_dir).await.is_some());

    // A valid session from another storage root may be resumed, but deletion
    // must fail before dropping its live registration or touching its files.
    let outside_store = tempfile::tempdir().unwrap();
    let outside = crate::core::SessionStore::new(
        outside_store.path(),
        workspace_b.path(),
        crate::domain::new_session_id(),
    )
    .unwrap();
    outside
        .append_history(
            crate::domain::new_thread_id(),
            None,
            &[crate::model_standard::CanonicalMessage::text(
                crate::model_standard::MessageRole::User,
                "outside storage",
            )],
        )
        .await
        .unwrap();
    command_value(
        post_at(
            &state,
            "/resume",
            json!({"session_dir":outside.session_dir()}),
        )
        .await,
    );
    assert!(matches!(
        post_at(
            &state,
            "/delete-session",
            json!({"session_dir":outside.session_dir()})
        )
        .await,
        StdioOutput::Response { ok: false, .. }
    ));
    assert!(outside.session_dir().exists());
    assert!(
        state
            .server_for_session_dir(outside.session_dir())
            .await
            .is_some()
    );
    let count = state.all_servers().await.len();
    assert!(matches!(
        post_at(
            &state,
            "/new-session",
            json!({"source_session_dir":"/missing-session"})
        )
        .await,
        StdioOutput::Response { ok: false, .. }
    ));
    assert_eq!(state.all_servers().await.len(), count);
    for server in state.all_servers().await {
        server.shutdown().await;
    }
}
