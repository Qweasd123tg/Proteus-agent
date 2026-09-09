use super::*;
use super::{
    oauth::{OAuthClient, Pkce},
    store::Credentials,
};
use crate::adapters::test_http;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::json;

fn credentials(expires_at: u64) -> Credentials {
    Credentials {
        access_token: "fixture-old-access".into(),
        refresh_token: "fixture-old-refresh".into(),
        account_id: "fixture-account".into(),
        expires_at,
    }
}

fn token_response() -> String {
    json!({"access_token": "fixture-new-access", "refresh_token": "fixture-new-refresh", "expires_in": 3600,
        "id_token": format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(json!({"https://api.openai.com/auth": {"chatgpt_account_id": "fixture-account"}}).to_string()))}).to_string()
}

#[tokio::test]
async fn browser_pkce_rejects_forged_state_then_exchanges_the_real_callback() {
    let (issuer, server) =
        test_http::server(vec![(200, "application/json", token_response())]).await;
    let oauth = OAuthClient::new(&issuer).unwrap();
    let pkce = Pkce::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redirect = format!("http://{}/auth/callback", listener.local_addr().unwrap());
    let url = oauth.authorize_url(&redirect, &pkce).unwrap();
    let query = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(
        query["code_challenge"],
        URL_SAFE_NO_PAD.encode(ring::digest::digest(
            &ring::digest::SHA256,
            pkce.verifier.as_bytes()
        ))
    );
    let expected_verifier = pkce.verifier.clone();
    let state = pkce.state.clone();
    let callback_redirect = redirect.clone();
    let callback = tokio::spawn(async move {
        login::browser_callback(listener, &oauth, &pkce, &callback_redirect)
            .await
            .unwrap()
    });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(
        http.get(format!("{redirect}?code=forged&state=wrong"))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        http.get(format!("{redirect}?code=real-code&state={state}"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let credential = callback.await.unwrap();
    assert_eq!(credential.account_id, "fixture-account");
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].target, "POST /oauth/token HTTP/1.1");
    let form = reqwest::Url::parse(&format!("http://localhost/?{}", requests[0].body)).unwrap();
    let pairs = form
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(pairs["code"], "real-code");
    assert_eq!(pairs["code_verifier"], expected_verifier);
    assert_eq!(pairs["redirect_uri"], redirect);
}

#[tokio::test]
async fn device_login_exchanges_code_with_the_device_redirect() {
    let (issuer, server) = test_http::server(vec![
        (
            200,
            "application/json",
            json!({"device_auth_id": "device-id", "user_code": "fixture-code", "interval": "1"})
                .to_string(),
        ),
        (
            200,
            "application/json",
            json!({"authorization_code": "device-code", "code_verifier": "device-verifier"})
                .to_string(),
        ),
        (200, "application/json", token_response()),
    ])
    .await;
    let credential = login::device(&OAuthClient::new(&issuer).unwrap())
        .await
        .unwrap();
    assert_eq!(credential.account_id, "fixture-account");
    let requests = server.await.unwrap();
    assert_eq!(
        requests[0].target,
        "POST /api/accounts/deviceauth/usercode HTTP/1.1"
    );
    assert!(requests[2].body.contains("code_verifier=device-verifier"));
    assert!(requests[2].body.contains("deviceauth%2Fcallback"));
}

#[tokio::test]
async fn separate_clients_share_one_rotation_and_persist_without_exposing_secrets() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("chatgpt.json");
    store::write(&path, &credentials(1)).unwrap();
    let (issuer, server) =
        test_http::server(vec![(200, "application/json", token_response())]).await;
    let config = json!({"auth_file": path, "oauth_issuer": issuer});
    let first = CodexAuth::from_config(&config).unwrap();
    let second = CodexAuth::from_config(&config).unwrap();
    let (a, b) = tokio::join!(first.access(None), second.access(None));
    assert_eq!(a.unwrap().token, "fixture-new-access");
    assert_eq!(b.unwrap().token, "fixture-new-access");
    // A delayed 401 for an old token reuses the already refreshed credential.
    assert_eq!(
        first
            .access(Some("fixture-old-access".into()))
            .await
            .unwrap()
            .token,
        "fixture-new-access"
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .body
            .contains("refresh_token=fixture-old-refresh")
    );
    assert_eq!(
        store::read(&path).unwrap().unwrap().refresh_token,
        "fixture-new-refresh"
    );
    assert!(!format!("{first:?}").contains("fixture-new-refresh"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn canceling_a_model_call_does_not_abandon_an_inflight_rotation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("chatgpt.json");
    store::write(&path, &credentials(1)).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let auth = CodexAuth::from_config(&json!({"auth_file": path, "oauth_issuer": format!("http://{}", listener.local_addr().unwrap())})).unwrap();
    let caller = tokio::spawn(async move { auth.access(None).await.map(|_| ()) });
    let (mut socket, _) = listener.accept().await.unwrap();
    let _request = test_http::read(&mut socket).await;
    caller.abort();
    let _ = caller.await;
    test_http::reply(&mut socket, 200, "application/json", &token_response()).await;
    let _lock = tokio::time::timeout(Duration::from_secs(3), store::lock(&path))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store::read(&path).unwrap().unwrap().refresh_token,
        "fixture-new-refresh"
    );
    store::remove(&path).unwrap();
    assert!(store::read(&path).unwrap().is_none());
}

#[tokio::test]
async fn refresh_failure_is_not_retried_and_does_not_replace_the_saved_login() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("chatgpt.json");
    store::write(&path, &credentials(1)).unwrap();
    let (issuer, server) = test_http::server(vec![(
        401,
        "application/json",
        json!({"error": "refresh_token_reused", "secret": "fixture-secret"}).to_string(),
    )])
    .await;
    let auth = CodexAuth::from_config(&json!({"auth_file": path, "oauth_issuer": issuer})).unwrap();
    let error = auth.access(None).await.err().unwrap().to_string();
    assert!(error.contains("login"));
    assert!(!error.contains("fixture-secret"));
    assert_eq!(server.await.unwrap().len(), 1);
    assert_eq!(
        store::read(&path).unwrap().unwrap().refresh_token,
        "fixture-old-refresh"
    );
    std::fs::write(&path, r#"{"tokens": {"access_token":"legacy"}}"#).unwrap();
    assert!(store::read(&path).is_err());
}
