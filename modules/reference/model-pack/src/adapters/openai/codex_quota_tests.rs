use super::*;
use crate::adapters::test_http;

fn quota_response() -> Value {
    json!({"plan_type":"plus", "rate_limit": {
        "allowed":true, "limit_reached":false,
        "primary_window":{"used_percent":27,"limit_window_seconds":18000,"reset_at":1900000000,"reset_after_seconds":42},
        "secondary_window":{"used_percent":63,"limit_window_seconds":604800,"reset_at":1900400000,"reset_after_seconds":4242}
    }, "additional_rate_limits":[{"metered_feature":"another-model", "limit_name":"Another model", "rate_limit": {
        "allowed":false,"limit_reached":true,"primary_window":{"used_percent":105,"limit_window_seconds":900,"reset_at":1900000000,"reset_after_seconds":42}
    }}], "credits":{"has_credits":true,"unlimited":false,"balance":"12.50"},
    "account_id":"must-not-cross-boundary", "unknown_future_field":{}})
}

#[tokio::test]
async fn quota_maps_all_windows_and_coalesces_oauth_readers() {
    let root = tempfile::tempdir().unwrap();
    let auth_file = super::codex_tests::auth_file(root.path());
    let (url, server) = test_http::server(vec![(
        200,
        "application/json",
        quota_response().to_string(),
    )])
    .await;
    let client = OpenAiResponsesClient::from_codex_config(
        json!({"auth_file":auth_file,"quota_url":format!("{url}/wham/usage")}),
    )
    .unwrap();
    let (first, second) = tokio::join!(client.quota(), client.quota());
    let snapshot = first.unwrap().unwrap();
    assert_eq!(Some(snapshot.clone()), second.unwrap());
    assert_eq!(snapshot.plan.as_deref(), Some("plus"));
    assert_eq!(snapshot.buckets[0].windows[0].used_percent, 27.0);
    assert_eq!(
        snapshot.buckets[0].windows[1].duration_seconds,
        Some(604800)
    );
    assert_eq!(snapshot.buckets[1].id, "another-model");
    assert_eq!(snapshot.buckets[1].windows[0].used_percent, 105.0);
    assert_eq!(snapshot.buckets[1].limit_reached, Some(true));
    assert_eq!(
        snapshot.credits.as_ref().unwrap().balance.as_deref(),
        Some("12.50")
    );
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains("must-not-cross-boundary"));
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].target, "GET /wham/usage HTTP/1.1");
    assert_eq!(
        requests[0].headers["authorization"],
        "Bearer fixture-access"
    );
    assert_eq!(requests[0].headers["chatgpt-account-id"], "fixture-account");
    assert!(requests[0].body.is_empty());
}

#[tokio::test]
async fn quota_refreshes_once_and_reports_errors_without_stale_or_body_leaks() {
    let root = tempfile::tempdir().unwrap();
    let auth_file = super::codex_tests::auth_file(root.path());
    let (url, server) = test_http::server(vec![
        (401, "application/json", "{}".into()),
        (
            200,
            "application/json",
            json!({"access_token":"rotated","refresh_token":"next","expires_in":3600}).to_string(),
        ),
        (200, "application/json", quota_response().to_string()),
        (
            429,
            "application/json",
            json!({"error":"do-not-leak"}).to_string(),
        ),
        (
            200,
            "application/json",
            json!({"plan_type":"free","rate_limit":null,"credits":null}).to_string(),
        ),
    ])
    .await;
    let client = OpenAiResponsesClient::from_codex_config(
        json!({"auth_file":auth_file,"quota_url":format!("{url}/usage"),"oauth_issuer":url}),
    )
    .unwrap();
    client.quota().await.unwrap();
    client.quota_cache.lock().await.as_mut().unwrap().0 -= std::time::Duration::from_secs(31);
    let error = format!("{:#}", client.quota().await.unwrap_err());
    assert!(error.contains("429"));
    assert!(!error.contains("do-not-leak"));
    let snapshot = client.quota().await.unwrap().unwrap();
    assert_eq!(snapshot.plan.as_deref(), Some("free"));
    assert!(snapshot.buckets[0].windows.is_empty());
    assert_eq!(snapshot.buckets[0].allowed, None);
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[1].target, "POST /oauth/token HTTP/1.1");
    assert_eq!(requests[2].headers["authorization"], "Bearer rotated");
}

#[tokio::test]
async fn quota_rejects_malformed_data_and_api_key_models_report_unsupported() {
    assert!(
        OpenAiResponsesClient::from_provider_config(json!({}))
            .unwrap()
            .quota()
            .await
            .unwrap()
            .is_none()
    );
    for value in [
        json!({"quota_url":4}),
        json!({"quota_url":"http://public.example/usage"}),
    ] {
        assert!(OpenAiResponsesClient::from_codex_config(value).is_err());
    }
    let root = tempfile::tempdir().unwrap();
    let auth_file = super::codex_tests::auth_file(root.path());
    let mut bad = quota_response();
    bad["rate_limit"]["primary_window"]["used_percent"] = json!(-1);
    let (url, server) = test_http::server(vec![
        (200, "application/json", bad.to_string()),
        (
            200,
            "application/json",
            json!({"plan_type":"secret-in-malformed-body"}).to_string(),
        ),
    ])
    .await;
    let client =
        OpenAiResponsesClient::from_codex_config(json!({"auth_file":auth_file,"quota_url":url}))
            .unwrap();
    assert!(
        client
            .quota()
            .await
            .unwrap_err()
            .to_string()
            .contains("non-negative")
    );
    // A valid plan without windows is authoritative absence, never 100% remaining.
    let empty = client.quota().await.unwrap().unwrap();
    assert!(empty.buckets[0].windows.is_empty());
    server.await.unwrap();
}
