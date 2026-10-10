use super::*;

#[tokio::test]
async fn route_config_builder_returns_editable_module_slots() {
    let config_dir = tempfile::tempdir().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let mut config = crate::test_model::config();
    config.tools.enabled = vec!["git_status".into()];
    config.components.insert(
        "tool-suite".into(),
        serde_json::from_value(json!({
            "command": crate::test_model::reference_module(),
            "description": "Example multi-pack plugin",
            "exports": {"tool": {
                "git_tools": {"description": "Git pack"},
                "file_tools": {}
            }}
        }))
        .unwrap(),
    );
    config.components.insert("probe-component".into(), serde_json::from_value(json!({
        "command": "sh",
        "args": [PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/process_tool.sh"), "probe.pack", "probe-component"],
        "exports": {"tool": {"probe.pack": {}}}
    })).unwrap());
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let server = AgentAppServer::launch(config, config_dir.path().into(), Some(&config_path))
        .await
        .unwrap();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;

    let response = route_request(
        state.clone(),
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

    let typed: proteus_contracts::app_protocol::config_builder::ConfigBuilderSnapshot =
        serde_json::from_slice(&bytes).expect("canonical builder DTO");
    let suite = typed
        .plugins
        .iter()
        .find(|plugin| plugin.id == "tool-suite")
        .unwrap();
    assert_eq!(
        suite.tool_packs.len(),
        2,
        "multiple packs share one component"
    );
    assert_eq!(
        suite
            .tool_packs
            .iter()
            .find(|pack| pack.id == "git_tools")
            .unwrap()
            .tools,
        ["git_diff", "git_status"]
    );
    assert_eq!(
        suite
            .exports
            .iter()
            .find(|export| export.id == "git_tools")
            .unwrap()
            .description
            .as_deref(),
        Some("Git pack")
    );
    let git_diff = typed
        .tools
        .iter()
        .find(|tool| tool.name == "git_diff")
        .unwrap();
    assert_eq!(git_diff.owner.as_ref().unwrap().component_id, "tool-suite");
    assert_eq!(git_diff.owner.as_ref().unwrap().module_id, "git_tools");
    assert!(!git_diff.enabled && !git_diff.registered && !git_diff.runtime_managed);
    let probe = typed
        .plugins
        .iter()
        .find(|plugin| plugin.id == "probe-component")
        .unwrap();
    assert!(
        probe.exports[0]
            .config_schema
            .as_ref()
            .unwrap()
            .fields
            .is_empty()
    );
    assert_eq!(probe.tool_packs[0].tools, ["detached_probe"]);
    let (runtime, _, _, _) = server.runtime.configuration_view().await;
    assert!(runtime.registry.tools.get("git_status").is_some());
    assert!(
        runtime.registry.tools.get("git_diff").is_none(),
        "inventory must not register disabled tools"
    );
    assert!(runtime.registry.tools.get("detached_probe").is_none());
    let topology = server.topology_snapshot().await;
    assert!(
        !topology.tools.iter().any(|tool| tool.name == "git_diff"),
        "execution topology must not advertise inventory as registered"
    );

    // Pack switches save the ordinary enabled list; execution and cold config
    // must follow it without altering the component launch/authority surface.
    for enabled in [json!(["git_diff", "git_status"]), json!([])] {
        let response = route_request(
            state.clone(),
            authed_json_request(
                &session_uri("/config/builder", &server),
                json!({"tools_enabled": enabled}),
            ),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let saved: Value = serde_json::from_slice(&response_bytes(response).await).unwrap();
        assert_eq!(saved["tools_enabled"], enabled);
        assert_eq!(
            serde_json::to_value(
                AppConfig::load(Some(&config_path))
                    .await
                    .unwrap()
                    .tools
                    .enabled
            )
            .unwrap(),
            enabled
        );
        let (runtime, _, _, _) = server.runtime.configuration_view().await;
        assert_eq!(
            runtime.registry.tools.get("git_diff").is_some(),
            !enabled.as_array().unwrap().is_empty()
        );
        let suite = saved["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|plugin| plugin["id"] == "tool-suite")
            .unwrap();
        let export = suite["exports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|export| export["id"] == "git_tools")
            .unwrap();
        assert_eq!(export["active"], !enabled.as_array().unwrap().is_empty());
        assert_eq!(
            suite["tool_packs"].as_array().unwrap().len(),
            2,
            "disabled packs stay discoverable"
        );
    }

    server.shutdown().await;
}

#[tokio::test]
async fn route_config_history_keeps_replaced_states_for_rollback() {
    let (state, server, _config_dir) = test_state().await;
    let history = || async {
        let response = route_request(
            state.clone(),
            authed_get_request(&session_uri("/config/history", &server)),
        )
        .await
        .expect("history response");
        assert_eq!(response.status(), StatusCode::OK);
        let history: Value =
            serde_json::from_slice(&response_bytes(response).await).expect("history JSON");
        history["revisions"]
            .as_array()
            .expect("revisions")
            .iter()
            .map(|revision| revision["state"].clone())
            .collect::<Vec<_>>()
    };
    let save = |body: Value| async {
        let response = route_request(
            state.clone(),
            authed_json_request(&session_uri("/config/builder", &server), body),
        )
        .await
        .expect("builder response");
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice::<Value>(&response_bytes(response).await).expect("builder JSON")
    };
    assert!(history().await.is_empty());

    save(json!({"permission_mode": "auto"})).await;
    save(json!({"permission_mode": "auto"})).await;
    let original = history().await;
    assert_eq!(
        original.len(),
        1,
        "an unchanged save has nothing to roll back"
    );
    assert_eq!(original[0]["permission_mode"], "normal");

    save(json!({"permission_mode": "plan", "tools_enabled": []})).await;
    let states = history().await;
    assert_eq!(states.len(), 2);
    assert_eq!(states[0]["permission_mode"], "auto", "newest first");

    // Rollback is an ordinary builder save of a recorded state.
    let restored = &states[1];
    let snapshot = save(json!({
        "modules": {},
        "hooks": restored["hooks"],
        "module_config": restored["module_config"],
        "tools_enabled": restored["tools_enabled"],
        "active_provider": restored["active_provider"],
        "permission_mode": restored["permission_mode"],
    }))
    .await;
    assert_eq!(snapshot["permission_mode"], "normal");
    assert_eq!(snapshot["tools_enabled"], restored["tools_enabled"]);
    assert_eq!(server.permission_mode().await, PermissionMode::Normal);
    let states = history().await;
    assert_eq!(states.len(), 3);
    assert_eq!(
        states[0]["permission_mode"], "plan",
        "rollback is reversible"
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
            + &crate::test_model::toml_component()
            + &format!(
                "\n[components.test-patch]\ncommand = {}\n[components.test-patch.exports.tool.direct_patch]\n[components.test-patch.exports.tool.rg_search]\n",
                serde_json::to_string(&crate::test_model::reference_module()).unwrap()
            ),
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
    let model_ref = server.runtime.model_ref().await.unwrap();
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
    assert_eq!(persisted.active_provider.as_deref(), Some("fake"));
    assert!(persisted.providers.contains_key("fake"));

    server.shutdown().await;
}
