use super::*;

#[tokio::test]
async fn route_history_can_read_requested_cold_session_without_changing_live_registry() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let saved_session_id = crate::domain::new_session_id();
    let saved_store =
        crate::core::SessionStore::new(config_dir.path(), cwd.path(), saved_session_id)
            .expect("session store");
    saved_store
        .append_history(
            crate::domain::new_thread_id(),
            None,
            &[crate::model_standard::CanonicalMessage::text(
                crate::model_standard::MessageRole::User,
                "saved cold history",
            )],
        )
        .await
        .expect("append saved history");
    let server = AgentAppServer::launch(
        crate::test_model::config(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .await
    .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;

    let response = route_request(
        state.clone(),
        authed_get_request(&format!(
            "/history?session_dir={}",
            saved_store.session_dir().display()
        )),
    )
    .await
    .expect("history response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response_bytes(response).await;
    let transcript: Vec<Value> = serde_json::from_slice(&bytes).expect("history JSON");
    assert_eq!(transcript.len(), 1);
    assert_eq!(
        transcript[0].get("role").and_then(Value::as_str),
        Some("user")
    );
    assert_eq!(
        transcript[0].get("text").and_then(Value::as_str),
        Some("saved cold history")
    );
    assert!(
        state
            .server_for_session_dir(saved_store.session_dir())
            .await
            .is_none()
    );
    assert!(
        state
            .server_for_session_dir(
                server
                    .session_dir_path()
                    .expect("live session dir")
                    .as_path(),
            )
            .await
            .is_some()
    );

    server.shutdown().await;
}

#[tokio::test]
async fn route_context_can_read_requested_cold_session_without_changing_live_registry() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let saved_session_id = crate::domain::new_session_id();
    let saved_store =
        crate::core::SessionStore::new(config_dir.path(), cwd.path(), saved_session_id)
            .expect("session store");
    saved_store
        .append_history(
            crate::domain::new_thread_id(),
            None,
            &[crate::model_standard::CanonicalMessage::text(
                crate::model_standard::MessageRole::User,
                "saved cold context",
            )],
        )
        .await
        .expect("append saved history");
    let server = AgentAppServer::launch(
        crate::test_model::config(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .await
    .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;

    let response = route_request(
        state.clone(),
        authed_get_request(&format!(
            "/context?session_dir={}",
            saved_store.session_dir().display()
        )),
    )
    .await
    .expect("context response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response_bytes(response).await;
    let snapshot: Value = serde_json::from_slice(&bytes).expect("context JSON");
    assert_eq!(
        snapshot
            .pointer("/history/messages")
            .and_then(Value::as_u64),
        Some(1)
    );
    let expected_session_id = saved_session_id.to_string();
    assert_eq!(
        snapshot.get("session_id").and_then(Value::as_str),
        Some(expected_session_id.as_str())
    );
    assert!(
        snapshot
            .get("diagnostics")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    );
    assert!(
        state
            .server_for_session_dir(saved_store.session_dir())
            .await
            .is_none()
    );
    assert!(
        state
            .server_for_session_dir(
                server
                    .session_dir_path()
                    .expect("live session dir")
                    .as_path(),
            )
            .await
            .is_some()
    );

    server.shutdown().await;
}
