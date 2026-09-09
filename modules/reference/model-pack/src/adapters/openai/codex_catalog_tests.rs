use super::*;
use crate::adapters::test_http;

fn remote_model(id: &str, priority: i64, hidden: bool, efforts: &[&str]) -> Value {
    json!({"slug": id, "display_name": format!("Display {id}"), "description": "fixture",
        "visibility": if hidden { "hide" } else { "list" }, "priority": priority,
        "default_reasoning_level": efforts.first(),
        "supported_reasoning_levels": efforts.iter().map(|effort| json!({"effort": effort, "description": "level"})).collect::<Vec<_>>(),
        "base_instructions": "must not enter the canonical catalog"})
}

#[tokio::test]
async fn catalog_fetches_all_models_and_per_model_efforts_with_oauth_and_shared_cache() {
    let root = tempfile::tempdir().unwrap();
    let auth_file = super::codex_tests::auth_file(root.path());
    let body = json!({"models": [remote_model("hidden-model", 2, true, &["medium"]),
        remote_model("new-model", 1, false, &["none", "high", "ultra", "future-effort"])]});
    let (url, server) = test_http::server(vec![(200, "application/json", body.to_string())]).await;
    let client =
        OpenAiResponsesClient::from_codex_config(json!({"auth_file": auth_file, "base_url": url}))
            .unwrap();
    let (first, second) = tokio::join!(client.catalog(), client.catalog());
    let catalog = first.unwrap().unwrap();
    assert_eq!(Some(catalog.clone()), second.unwrap());
    assert_eq!(catalog.models[0].id, "new-model");
    assert_eq!(
        catalog.models[0].reasoning_efforts,
        ["none", "high", "ultra", "future-effort"]
    );
    assert_eq!(
        catalog.models[0].default_reasoning_effort.as_deref(),
        Some("none")
    );
    assert!(catalog.models[1].hidden);
    assert!(
        !serde_json::to_string(&catalog)
            .unwrap()
            .contains("base_instructions")
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].target,
        "GET /models?client_version=0.0.0 HTTP/1.1"
    );
    assert_eq!(
        requests[0].headers["authorization"],
        "Bearer fixture-access"
    );
    assert_eq!(requests[0].headers["chatgpt-account-id"], "fixture-account");
    assert!(requests[0].body.is_empty());
}

#[tokio::test]
async fn catalog_refreshes_rejected_auth_and_reports_failures_without_stale_fallback() {
    let root = tempfile::tempdir().unwrap();
    let auth_file = super::codex_tests::auth_file(root.path());
    let body = json!({"models": [remote_model("model", 1, false, &["low"])]});
    let (url, server) = test_http::server(vec![
        (401, "application/json", "{}".into()),
        (
            200,
            "application/json",
            json!({"access_token": "rotated", "refresh_token": "next", "expires_in": 3600})
                .to_string(),
        ),
        (200, "application/json", body.to_string()),
        (
            429,
            "application/json",
            "{\"secret\":\"never-log-this\"}".into(),
        ),
        (200, "application/json", json!({"models": []}).to_string()),
    ])
    .await;
    let client = OpenAiResponsesClient::from_codex_config(
        json!({"auth_file": auth_file, "base_url": url, "oauth_issuer": url}),
    )
    .unwrap();
    assert_eq!(client.catalog().await.unwrap().unwrap().models.len(), 1);
    client.catalog_cache.lock().await.as_mut().unwrap().0 -= std::time::Duration::from_secs(301);
    let error = format!("{:#}", client.catalog().await.unwrap_err());
    assert!(error.contains("429"));
    assert!(!error.contains("never-log-this"));
    assert!(client.catalog().await.unwrap().unwrap().models.is_empty());
    let requests = server.await.unwrap();
    assert_eq!(requests[1].target, "POST /oauth/token HTTP/1.1");
    assert_eq!(requests[2].headers["authorization"], "Bearer rotated");
}

#[tokio::test]
async fn catalog_rejects_invalid_metadata_and_api_adapter_does_not_read_oauth() {
    let api = OpenAiResponsesClient::from_provider_config(json!({})).unwrap();
    assert_eq!(api.catalog().await.unwrap(), None);
    let root = tempfile::tempdir().unwrap();
    let auth_file = super::codex_tests::auth_file(root.path());
    let mut invalid = remote_model("model", 0, false, &["high"]);
    invalid["default_reasoning_level"] = json!("unknown");
    let (url, server) = test_http::server(vec![(
        200,
        "application/json",
        json!({"models": [invalid]}).to_string(),
    )])
    .await;
    let client =
        OpenAiResponsesClient::from_codex_config(json!({"auth_file": auth_file, "base_url": url}))
            .unwrap();
    assert!(
        client
            .catalog()
            .await
            .unwrap_err()
            .to_string()
            .contains("default effort")
    );
    server.await.unwrap();
}
