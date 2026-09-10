use super::*;

#[tokio::test]
async fn route_config_builder_returns_editable_module_slots() {
    let (state, server, _config_dir) = test_state().await;

    let response = route_request(
        state,
        authed_get_request(&session_uri("/config/builder", &server)),
    )
    .await
    .expect("config builder response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response_bytes(response).await;
    let snapshot: Value = serde_json::from_slice(&bytes).expect("builder JSON");
    assert_eq!(
        snapshot.get("writable").and_then(Value::as_bool),
        Some(true)
    );
    let slots = snapshot
        .get("slots")
        .and_then(Value::as_array)
        .expect("builder slots");
    let slot_ids = slots
        .iter()
        .filter_map(|slot| slot.get("id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(
        slot_ids,
        vec![
            "workflow",
            "context",
            "compactor",
            "tool_exposure",
            "policy",
            "search",
            "patch",
            "memory",
        ]
    );
    assert!(
        slots
            .iter()
            .all(|slot| { slot.get("modules").and_then(Value::as_array).is_some() })
    );
    assert!(slots.iter().any(|slot| {
        slot.get("id").and_then(Value::as_str) == Some("workflow")
            && slot
                .get("modules")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
    }));
    assert!(!slots.iter().any(|slot| {
        matches!(
            slot.get("id").and_then(Value::as_str),
            Some("model" | "tool")
        )
    }));
    assert!(snapshot.get("tools_enabled").is_some_and(Value::is_array));
    assert!(snapshot.get("tools").is_some_and(Value::is_array));
    assert!(snapshot.get("providers").is_some_and(Value::is_array));
    assert_eq!(
        snapshot.get("permission_mode").and_then(Value::as_str),
        Some("normal")
    );
    assert_eq!(
        snapshot.get("permission_modes"),
        Some(&json!(["plan", "normal", "auto"]))
    );

    server.shutdown().await;
}

#[tokio::test]
async fn route_config_builder_persists_settings_and_reloads_runtime() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        r#"
active_provider = "fast"

[providers.fast]
provider = "fake"
model = "fake-fast"

[providers.smart]
provider = "fake"
model = "fake-smart"

"#
        .to_owned()
            + &crate::test_model::toml_component(),
    )
    .expect("write config");
    let config = {
        let raw = std::fs::read_to_string(&config_path).expect("read config");
        toml::from_str::<AppConfig>(&raw).expect("parse config")
    };
    let server = AgentAppServer::launch(config, cwd.path().to_path_buf(), Some(&config_path))
        .await
        .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;

    let response = route_request(
        state,
        authed_json_request(
            &session_uri("/config/builder", &server),
            json!({
                "modules": {},
                "tools_enabled": ["apply_patch", "search"],
                "active_provider": "smart",
                "permission_mode": "auto"
            }),
        ),
    )
    .await
    .expect("config builder save response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response_bytes(response).await;
    let snapshot: Value = serde_json::from_slice(&bytes).expect("builder JSON");
    assert_eq!(snapshot.get("active_modules"), Some(&json!([])));

    let written = std::fs::read_to_string(&config_path).expect("read config");
    assert!(
        written.contains("enabled = [\"apply_patch\", \"search\"]"),
        "{written}"
    );
    assert!(written.contains("active_provider = \"smart\""), "{written}");
    assert!(written.contains("mode = \"auto\""), "{written}");
    assert_eq!(
        snapshot.get("tools_enabled"),
        Some(&json!(["apply_patch", "search"]))
    );
    assert_eq!(
        snapshot.get("active_provider").and_then(Value::as_str),
        Some("smart")
    );
    assert_eq!(
        snapshot.get("permission_mode").and_then(Value::as_str),
        Some("auto")
    );
    assert!(
        snapshot
            .get("providers")
            .and_then(Value::as_array)
            .is_some_and(|providers| providers.iter().any(|provider| {
                provider.get("id").and_then(Value::as_str) == Some("smart")
                    && provider.get("active").and_then(Value::as_bool) == Some(true)
            }))
    );
    assert_eq!(server.permission_mode().await, PermissionMode::Auto);
    let model_ref = server.runtime.model_ref().await;
    assert_eq!(model_ref.model, "fake-smart");

    let summary = server.config_summary().await;
    assert_eq!(summary.get("modules"), Some(&json!([])));
    assert!(
        summary
            .get("module_epoch")
            .and_then(Value::as_u64)
            .is_some_and(|epoch| epoch > 0)
    );

    server.shutdown().await;
}

#[tokio::test]
async fn route_config_builder_creates_complete_provider_config() {
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

    let response = route_request(
        state,
        authed_json_request(
            &session_uri("/config/builder", &server),
            json!({
                "modules": {},
                "module_config": {}
            }),
        ),
    )
    .await
    .expect("config builder save response");

    let status = response.status();
    let body = response_bytes(response).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let persisted = AppConfig::load(Some(&config_path))
        .await
        .expect("persisted config must load");
    assert_eq!(persisted.active_provider, "fake");
    assert!(persisted.providers.contains_key("fake"));

    server.shutdown().await;
}
