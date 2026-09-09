use super::*;
use crate::adapters::test_http;
use futures_util::StreamExt;
use std::time::{SystemTime, UNIX_EPOCH};

fn auth_file(root: &std::path::Path) -> std::path::PathBuf {
    let path = root.join("chatgpt.json");
    std::fs::write(&path, json!({
        "access_token": "fixture-access", "refresh_token": "fixture-refresh", "account_id": "fixture-account",
        "expires_at": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600,
    }).to_string()).unwrap();
    path
}

fn request() -> CanonicalModelRequest {
    CanonicalModelRequest::new(
        ModelRef::new("subscription", "fixture-model"),
        vec![CanonicalMessage::text(MessageRole::User, "hello")],
    )
}

fn response() -> Value {
    json!({"status": "completed", "output": [{"type": "function_call", "call_id": "fixture-call", "name": "read_file", "arguments": "{\"path\":\"probe.txt\"}"}], "usage": {"input_tokens": 12, "output_tokens": 3}})
}

#[tokio::test]
async fn subscription_uses_oauth_and_sse_for_stream_and_complete() {
    let root = tempfile::tempdir().unwrap();
    let auth_file = auth_file(root.path());
    for stream in [true, false] {
        let (url, server) = test_http::server(vec![test_http::sse(response())]).await;
        let client = OpenAiResponsesClient::from_codex_config(
            json!({"auth_file": auth_file, "base_url": url, "stream": stream}),
        )
        .unwrap();
        let mut request = request();
        request.limits.max_output_tokens = Some(123);
        let mut events = client.stream(request).await.unwrap();
        let mut terminal = None;
        while let Some(event) = events.next().await {
            if let ModelStreamEvent::Response { response } = event.unwrap() {
                terminal = Some(response);
            }
        }
        let terminal = terminal.unwrap();
        assert_eq!(terminal.tool_calls[0].name, "read_file");
        let requests = server.await.unwrap();
        let request = &requests[0];
        assert_eq!(request.target, "POST /responses HTTP/1.1");
        assert_eq!(request.headers["authorization"], "Bearer fixture-access");
        assert_eq!(request.headers["chatgpt-account-id"], "fixture-account");
        assert_eq!(request.headers["originator"], "proteus");
        let body: Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert_eq!(body["instructions"], "");
        assert!(body.get("max_output_tokens").is_none());
    }
}

#[tokio::test]
async fn unauthorized_refreshes_once_while_quota_errors_do_not_retry() {
    let root = tempfile::tempdir().unwrap();
    let auth_file = auth_file(root.path());
    let (url, server) = test_http::server(vec![
        (401, "application/json", "{}".into()),
        (200, "application/json", json!({"access_token": "rotated-access", "refresh_token": "rotated-refresh", "expires_in": 3600}).to_string()),
        test_http::sse(response()),
        (429, "application/json", json!({"error":{"message":"subscription allowance exhausted", "type":"usage_limit_reached"}}).to_string()),
    ]).await;
    let client = OpenAiResponsesClient::from_codex_config(
        json!({"auth_file": auth_file, "base_url": url, "oauth_issuer": url, "stream": true}),
    )
    .unwrap();
    let mut stream = client.stream(request()).await.unwrap();
    while let Some(event) = stream.next().await {
        event.unwrap();
    }
    let error = client.stream(request()).await.err().unwrap();
    assert!(
        error
            .to_string()
            .contains("subscription allowance exhausted")
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[1].target, "POST /oauth/token HTTP/1.1");
    assert_eq!(
        requests[2].headers["authorization"],
        "Bearer rotated-access"
    );
    assert_eq!(
        requests[3].headers["authorization"],
        "Bearer rotated-access"
    );
}

#[test]
fn subscription_rejects_api_credentials_and_nonstream_fallback_without_reading_auth() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("not-logged-in.json");
    assert!(OpenAiResponsesClient::from_codex_config(json!({"auth_file": missing})).is_ok());
    for (key, value) in [
        ("api_key", json!("fixture-api-key")),
        ("stream_error_fallback", json!(true)),
        ("prompt_cache_retention", json!("24h")),
        ("auth_flie", json!("typo.json")),
        ("max_input_tokens", json!("large")),
        ("capabilities", json!({"unknown": true})),
    ] {
        let mut config = json!({"auth_file": missing});
        config[key] = value;
        assert!(OpenAiResponsesClient::from_codex_config(config).is_err());
    }
}
