use proteus_core::core::{AppConfig, ModuleCatalog};
use serde_json::json;

#[test]
fn one_reference_worker_supports_independent_model_exports_using_the_same_implementation() {
    let cwd = tempfile::tempdir().unwrap();
    let config: AppConfig = serde_json::from_value(json!({
        "active_provider": "fast",
        "providers": {
            "fast": {"provider": "endpoint_a", "model": "first", "stream": true},
            "large": {"provider": "endpoint_b", "model": "second", "stream": false}
        },
        "components": {"models": {
            "command": env!("CARGO_BIN_EXE_proteus-reference-worker"),
            "exports": {"model": {"endpoint_a": {}, "endpoint_b": {}}}
        }},
        "module_config": {"model": {
            "endpoint_a": {"implementation": "openai", "max_input_tokens": 1234},
            "endpoint_b": {"implementation": "openai", "max_input_tokens": 5678}
        }}
    }))
    .unwrap();
    let catalog = ModuleCatalog::from_config(&config).unwrap();
    for (profile, max_tokens) in [("fast", 1234), ("large", 5678)] {
        let model_config = config.providers[profile].to_model_config().unwrap();
        let model = catalog
            .build_model_adapter(&model_config, cwd.path())
            .unwrap();
        assert_eq!(
            model
                .capabilities(&model_config.model_ref())
                .max_input_tokens,
            Some(max_tokens)
        );
    }
}

#[test]
fn reference_model_requires_explicit_implementation_in_opaque_module_config() {
    let cwd = tempfile::tempdir().unwrap();
    let config: AppConfig = serde_json::from_value(json!({
        "active_provider": "fake", "providers": {"fake": {"provider": "fake"}},
        "components": {"model": {"command": env!("CARGO_BIN_EXE_proteus-reference-worker"),
            "exports": {"model": {"fake": {}}}}}
    }))
    .unwrap();
    let catalog = ModuleCatalog::from_config(&config).unwrap();
    assert!(
        catalog
            .build_model_adapter(&config.active_model_config().unwrap(), cwd.path())
            .is_err()
    );
}

#[tokio::test]
async fn subscription_catalog_crosses_real_worker_and_updates_app_selection() {
    use proteus_core::app_server::AgentAppServer;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let cwd = tempfile::tempdir().unwrap();
    let auth_file = cwd.path().join("chatgpt.json");
    std::fs::write(
        &auth_file,
        json!({"access_token": "catalog-access", "refresh_token": "catalog-refresh",
        "account_id": "catalog-account", "expires_at": u64::MAX / 2})
        .to_string(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let remote = tokio::spawn(async move {
        for quota_request in [false, true] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(bytes).unwrap().to_lowercase();
            assert!(request.starts_with(if quota_request {
                "get /wham/usage http/1.1"
            } else {
                "get /models?client_version=0.0.0 http/1.1"
            }));
            assert!(request.contains("authorization: bearer catalog-access"));
            assert!(request.contains("chatgpt-account-id: catalog-account"));
            let body = if quota_request {
                json!({"plan_type":"plus", "rate_limit": {"allowed":true,"limit_reached":false,"primary_window":{"used_percent":25,"limit_window_seconds":18000,"reset_at":1900000000}}}).to_string()
            } else {
                json!({"models": [
            {"slug": "discovered-a", "display_name": "A", "visibility": "list", "priority": 1,
                "supported_reasoning_levels": [{"effort":"high"},{"effort":"ultra"}], "default_reasoning_level": "high"},
            {"slug": "discovered-b", "display_name": "B", "visibility": "hide", "priority": 2,
                "supported_reasoning_levels": [{"effort":"low"}], "default_reasoning_level": "low"}
        ]}).to_string()
            };
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
    });
    let config: AppConfig = serde_json::from_value(json!({
        "active_provider": "subscription", "providers": {"subscription": {"provider": "arbitrary-export", "model": "discovered-a"}},
        "components": {"models": {"command": env!("CARGO_BIN_EXE_proteus-reference-worker"), "exports": {"model": {"arbitrary-export": {}}}}},
        "module_config": {"model": {"arbitrary-export": {"implementation": "openai_codex", "auth_file": auth_file, "base_url": base_url, "quota_url": format!("{base_url}/wham/usage")}}}
    })).unwrap();
    let server = AgentAppServer::launch(config, cwd.path().to_path_buf(), None)
        .await
        .unwrap();
    let summary = server.config_summary().await;
    assert!(summary["model_catalog_error"].is_null(), "{summary}");
    assert_eq!(summary["model_options"][1]["name"], "discovered-b");
    assert_eq!(summary["model_options"][1]["hidden"], true);
    assert_eq!(
        summary["reasoning"]["effort_options"],
        json!(["high", "ultra"])
    );
    server
        .set_reasoning_effort(Some("ultra".into()))
        .await
        .unwrap();
    server.set_model_name("discovered-b".into()).await.unwrap();
    let changed = server.config_summary().await;
    assert_eq!(changed["reasoning"]["effort_options"], json!(["low"]));
    assert_eq!(changed["reasoning"]["effort"], "low");
    assert!(
        server
            .set_reasoning_effort(Some("ultra".into()))
            .await
            .is_err()
    );
    let quota = server.model_quota().await.unwrap().unwrap();
    assert_eq!(quota.plan.as_deref(), Some("plus"));
    assert_eq!(quota.buckets[0].windows[0].used_percent, 25.0);
    assert_eq!(quota.buckets[0].windows[0].duration_seconds, Some(18000));
    remote.await.unwrap();
    server.shutdown().await;
}
