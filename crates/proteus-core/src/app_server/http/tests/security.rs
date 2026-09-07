use super::*;

#[test]
fn protected_endpoints_require_session_token_except_health_and_preflight() {
    let security = test_security();
    let protected = [
        (Method::GET, "/events"),
        (Method::GET, "/config"),
        (Method::GET, "/config/builder"),
        (Method::GET, "/inspect/topology"),
        (Method::GET, "/inspect/plan"),
        (Method::GET, "/inspect/topology.mmd"),
        (Method::GET, "/inspect/topology.map"),
        (Method::GET, "/inspect/topology.runtime"),
        (Method::GET, "/inspect/topology.runtime.mmd"),
        (Method::GET, "/sessions"),
        (Method::GET, "/sessions/current"),
        (Method::GET, "/history"),
        (Method::GET, "/context"),
        (Method::POST, "/request"),
        (Method::POST, "/send"),
        (Method::POST, "/send-async"),
        (Method::POST, "/approval"),
        (Method::POST, "/user-input"),
        (Method::POST, "/cancel"),
        (Method::POST, "/mode"),
        (Method::POST, "/model"),
        (Method::POST, "/effort"),
        (Method::POST, "/reasoning"),
        (Method::POST, "/config/builder"),
        (Method::POST, "/resume"),
        (Method::POST, "/new-session"),
        (Method::POST, "/delete-session"),
        (Method::POST, "/clear"),
        (Method::POST, "/reload-tools"),
        (Method::POST, "/shutdown"),
    ];

    for (method, path) in protected {
        assert!(
            endpoint_requires_auth(&method, path),
            "{method} {path} must require auth"
        );
        assert!(
            request_requires_session_token(&method, path, &security),
            "{method} {path} must require session token when token auth is enabled"
        );
    }
    assert!(!endpoint_requires_auth(&Method::GET, "/health"));
    assert!(!endpoint_requires_auth(&Method::OPTIONS, "/config"));
    assert!(!request_requires_session_token(
        &Method::GET,
        "/health",
        &security
    ));
    assert!(!request_requires_session_token(
        &Method::OPTIONS,
        "/config",
        &security
    ));
}

#[tokio::test]
async fn read_json_rejects_oversized_body() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/send")
        .body(Full::new(Bytes::from(vec![b' '; MAX_JSON_BODY_BYTES + 1])))
        .expect("request");

    let error = read_json::<Value, _>(request)
        .await
        .expect_err("oversized body should be rejected")
        .to_string();

    assert!(error.contains("within"), "{error}");
    assert!(error.contains(&MAX_JSON_BODY_BYTES.to_string()), "{error}");
}

#[test]
fn default_http_config_does_not_require_session_token() {
    let config = HttpServerConfig::default();
    config.validate().expect("loopback debug config");
    let security = HttpSecurity::from_config(&config);

    assert!(!config.require_session_token);
    assert!(!request_requires_session_token(
        &Method::GET,
        "/config",
        &security
    ));
}

#[test]
fn http_config_requires_non_empty_token_for_non_loopback_bind() {
    let mut config = HttpServerConfig {
        bind: "0.0.0.0:8787".parse().expect("bind"),
        ..HttpServerConfig::default()
    };
    assert!(
        config
            .validate()
            .expect_err("external bind without auth must fail")
            .to_string()
            .contains("requires --token")
    );

    config.require_session_token = true;
    config.session_token.clear();
    assert!(
        config
            .validate()
            .expect_err("empty required token must fail")
            .to_string()
            .contains("must not be empty")
    );

    config.session_token = "secret".to_owned();
    config.validate().expect("external authenticated bind");
}

#[test]
fn token_auth_accepts_authorization_and_query_tokens() {
    let security = test_security();
    let bearer_request = Request::builder()
        .uri("/config")
        .header(AUTHORIZATION, "Bearer session-secret")
        .body(())
        .expect("request");
    let query_request = Request::builder()
        .uri("/events?token=session-secret")
        .body(())
        .expect("request");
    assert!(request_has_valid_token(&bearer_request, &security));
    assert!(request_has_valid_token(&query_request, &security));
}

#[test]
fn token_auth_accepts_percent_encoded_event_source_query_token() {
    let security = HttpSecurity {
        session_token: Arc::from("session secret/%"),
        require_session_token: true,
        allowed_origins: Arc::from(default_allowed_origins().into_boxed_slice()),
    };
    let request = Request::builder()
        .uri("/events?token=session%20secret%2F%25")
        .body(())
        .expect("request");

    assert!(request_has_valid_token(&request, &security));
}

#[test]
fn token_auth_rejects_missing_and_invalid_tokens() {
    let security = test_security();
    let missing = Request::builder().uri("/config").body(()).expect("request");
    let removed_header = Request::builder()
        .uri("/config")
        .header("x-proteus-session", "session-secret")
        .body(())
        .expect("request");
    let removed_query_alias = Request::builder()
        .uri("/events?session=session-secret")
        .body(())
        .expect("request");
    let invalid_bearer = Request::builder()
        .uri("/config")
        .header(AUTHORIZATION, "Bearer wrong")
        .body(())
        .expect("request");
    let invalid_query = Request::builder()
        .uri("/events?token=wrong")
        .body(())
        .expect("request");

    assert!(!request_has_valid_token(&missing, &security));
    assert!(!request_has_valid_token(&removed_header, &security));
    assert!(!request_has_valid_token(&removed_query_alias, &security));
    assert!(!request_has_valid_token(&invalid_bearer, &security));
    assert!(!request_has_valid_token(&invalid_query, &security));
}

#[test]
fn origin_validation_allows_configured_origins() {
    let security = test_security();
    for origin in [
        "http://127.0.0.1:1420",
        "http://localhost:1420",
        "http://127.0.0.1:1421",
        "http://localhost:1421",
        "https://app.example.test",
    ] {
        let request = request_with_origin(Some(origin));
        let allowed = validate_origin(&request, &security).expect("allowed");
        assert_eq!(
            allowed.as_ref().and_then(|value| value.to_str().ok()),
            Some(origin)
        );
    }
    let request = request_with_origin(None);
    assert!(validate_origin(&request, &security).unwrap().is_none());
}

#[test]
fn origin_validation_rejects_untrusted_origins() {
    let security = test_security();
    for origin in [
        "https://evil.example.test",
        "null",
        "file://localhost/tmp/app.html",
        "http://127.0.0.1:5173",
        "http://[::1]:1420",
        "http://localhost.evil.example.test",
    ] {
        let request = request_with_origin(Some(origin));
        let response = validate_origin(&request, &security).expect_err("rejected");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

#[test]
fn options_response_adds_cors_headers_for_allowed_origin() {
    let request = Request::builder()
        .method(Method::OPTIONS)
        .uri("/config")
        .header(ORIGIN, "http://localhost:1420")
        .header("access-control-request-method", "POST")
        .body(())
        .expect("request");
    let origin = validate_origin(&request, &test_security()).expect("origin");
    let response = options_response(&request, origin);

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("http://localhost:1420")
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-headers")
            .and_then(|value| value.to_str().ok()),
        Some("authorization, content-type")
    );
}

#[tokio::test]
async fn route_rejects_missing_token_before_dispatching_protected_endpoint() {
    let (state, server) = test_state().await;
    let request = Request::builder()
        .method(Method::GET)
        .uri("/config")
        .body(empty_body())
        .expect("request");

    let response = route_request(state, request).await.expect("response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
    server.shutdown().await;
}

#[tokio::test]
async fn route_rejects_event_stream_without_token() {
    let (state, server) = test_state().await;
    let request = Request::builder()
        .method(Method::GET)
        .uri("/events")
        .body(empty_body())
        .expect("request");

    let response = route_request(state, request).await.expect("response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    server.shutdown().await;
}

#[tokio::test]
async fn route_rejects_mutating_endpoint_without_token() {
    let (state, server) = test_state().await;
    let request = Request::builder()
        .method(Method::POST)
        .uri("/send")
        .body(empty_body())
        .expect("request");

    let response = route_request(state, request).await.expect("response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    server.shutdown().await;
}

#[tokio::test]
async fn route_rejects_bad_origin_even_with_valid_token() {
    let (state, server) = test_state().await;
    let request = Request::builder()
        .method(Method::GET)
        .uri("/config")
        .header(ORIGIN, "https://evil.example.test")
        .header(AUTHORIZATION, "Bearer session-secret")
        .body(empty_body())
        .expect("request");

    let response = route_request(state, request).await.expect("response");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
    server.shutdown().await;
}

#[tokio::test]
async fn route_accepts_allowed_origin_and_never_uses_wildcard_cors() {
    let (state, server) = test_state().await;
    let request = Request::builder()
        .method(Method::GET)
        .uri("/config")
        .header(ORIGIN, "http://127.0.0.1:1420")
        .header(AUTHORIZATION, "Bearer session-secret")
        .body(empty_body())
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
    assert_ne!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("*")
    );
    server.shutdown().await;
}
