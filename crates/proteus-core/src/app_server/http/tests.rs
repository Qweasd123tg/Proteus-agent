use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use coding_workflow::CodingSingleLoopWorkflow;
use context_pack::SimpleContextBuilderPlugin;
use hyper::header::{AUTHORIZATION, ORIGIN};
use policy_pack::AskWritePolicyPlugin;
use proteus_contracts::{
    abi_stable::sabi_trait::TD_Opaque,
    contracts::{ApprovalCacheScope, Renderer_TO},
    plugin::{PluginApprovalPolicy_TO, PluginContextBuilder_TO, PluginWorkflow_TO},
};
use renderer_pack::PlainRendererPlugin;
use serde_json::Value;

use super::*;
use crate::contracts::{
    ApprovalResponse, CancellationToken, UserInputAnswer,
    UserInputRequest as ContractUserInputRequest, UserInputResponse,
};
use crate::core::{AppConfig, BuiltinModuleCatalog};
use crate::domain::{PermissionMode, ToolCall, new_call_id};

use super::config::default_allowed_origins;
use super::security::{
    endpoint_requires_auth, request_has_valid_token, request_requires_session_token,
    validate_origin,
};

fn empty_body() -> Full<Bytes> {
    Full::new(Bytes::new())
}

fn test_security() -> HttpSecurity {
    let mut allowed_origins = default_allowed_origins();
    allowed_origins.push("https://app.example.test".to_owned());
    HttpSecurity {
        session_token: Arc::from("session-secret"),
        require_session_token: true,
        allowed_origins: Arc::from(allowed_origins.into_boxed_slice()),
    }
}

fn request_with_origin(origin: Option<&str>) -> Request<()> {
    let mut builder = Request::builder().method(Method::GET).uri("/config");
    if let Some(origin) = origin {
        builder = builder.header(ORIGIN, origin);
    }
    builder.body(()).expect("request")
}

async fn test_state() -> (HttpAppState, AppServerHandle) {
    let cwd = tempfile::tempdir().expect("cwd");
    let server = AgentAppServer::launch(AppConfig::default(), cwd.path().to_path_buf(), None)
        .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());
    (state, server)
}

fn pending_approval_entry(
    approval_id: &str,
    responder: tokio::sync::oneshot::Sender<ApprovalResponse>,
) -> crate::app_server::PendingApprovalEntry {
    crate::app_server::PendingApprovalEntry {
        request: crate::app_server::AppApprovalRequest::new(
            approval_id.to_owned(),
            ToolCall::new(new_call_id(), "write_file", json!({ "path": "notes.txt" })),
            PathBuf::from("/workspace"),
            "test approval".to_owned(),
            None,
        ),
        responder,
    }
}

fn pending_user_input_entry(
    request_id: &str,
    responder: tokio::sync::oneshot::Sender<UserInputResponse>,
) -> crate::app_server::PendingUserInputEntry {
    crate::app_server::PendingUserInputEntry {
        request: ContractUserInputRequest::new(
            request_id.to_owned(),
            PathBuf::from("/workspace"),
            Vec::new(),
        ),
        responder,
    }
}

async fn dogfood_loop_state() -> (HttpAppState, AppServerHandle) {
    let cwd = tempfile::tempdir().expect("cwd");
    let server = AgentAppServer::launch_with_module_catalog(
        dogfood_loop_config(),
        cwd.path().to_path_buf(),
        None,
        dogfood_loop_catalog(),
    )
    .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());
    (state, server)
}

fn dogfood_loop_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.model.stream = false;
    config.modules.workflow = "coding.single_loop".to_owned();
    config.modules.context = "simple".to_owned();
    config.modules.policy = "ask_write".to_owned();
    config.modules.patch = "null".to_owned();
    config.modules.renderer = "plain".to_owned();
    config.tools.enabled = vec!["apply_patch".to_owned(), "request_user_input".to_owned()];
    config.module_config.insert(
        "policy".to_owned(),
        BTreeMap::from([(
            "ask_write".to_owned(),
            json!({
                "ask_before": ["apply_patch"],
                "allow": ["request_user_input"],
            }),
        )]),
    );
    config
}

fn dogfood_loop_catalog() -> BuiltinModuleCatalog {
    let mut catalog = BuiltinModuleCatalog::new();
    catalog
        .register_plugin_context_builder(
            "simple",
            PluginContextBuilder_TO::from_value(SimpleContextBuilderPlugin, TD_Opaque),
        )
        .expect("register test context builder");
    catalog
        .register_plugin_workflow(
            "coding.single_loop",
            PluginWorkflow_TO::from_value(CodingSingleLoopWorkflow::default(), TD_Opaque),
        )
        .expect("register test workflow");
    catalog
        .register_plugin_policy(
            "ask_write",
            PluginApprovalPolicy_TO::from_value(AskWritePolicyPlugin, TD_Opaque),
        )
        .expect("register test policy");
    catalog
        .register_plugin_renderer(
            "plain",
            Renderer_TO::from_value(PlainRendererPlugin, TD_Opaque),
        )
        .expect("register test renderer");
    catalog
}

fn json_body(value: Value) -> Full<Bytes> {
    Full::new(Bytes::from(
        serde_json::to_vec(&value).expect("test JSON serializes"),
    ))
}

async fn response_output(response: HttpResponse) -> StdioOutput {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("response should be protocol JSON")
}

async fn response_bytes(response: HttpResponse) -> Bytes {
    response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes()
}

fn authed_get_request(path: &str) -> Request<Full<Bytes>> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(ORIGIN, "http://127.0.0.1:1420")
        .header("x-proteus-session", "session-secret")
        .body(empty_body())
        .expect("request")
}

fn authed_json_request(path: &str, value: Value) -> Request<Full<Bytes>> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(ORIGIN, "http://127.0.0.1:1420")
        .header("x-proteus-session", "session-secret")
        .header(CONTENT_TYPE, "application/json")
        .body(json_body(value))
        .expect("request")
}

async fn wait_for_approval_request(
    event_rx: &mut broadcast::Receiver<AppServerEvent>,
) -> crate::app_server::AppApprovalRequest {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("approval request event should arrive")
            .expect("event stream should stay open");
        match event {
            AppServerEvent::ApprovalRequested { request } => return *request,
            AppServerEvent::Error { message } => {
                panic!("unexpected app-server error: {message}")
            }
            _ => {}
        }
    }
}

async fn wait_for_user_input_request(
    event_rx: &mut broadcast::Receiver<AppServerEvent>,
) -> ContractUserInputRequest {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("user-input request event should arrive")
            .expect("event stream should stay open");
        match event {
            AppServerEvent::UserInputRequested { request } => return *request,
            AppServerEvent::Error { message } => {
                panic!("unexpected app-server error: {message}")
            }
            _ => {}
        }
    }
}

async fn wait_for_transcript_text(
    server: &AppServerHandle,
    text: &str,
) -> Vec<crate::app_server::AppTranscriptMessage> {
    for _ in 0..50 {
        let transcript = server.transcript().await;
        if transcript.iter().any(|message| message.text == text) {
            return transcript;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    server.transcript().await
}

#[test]
fn protected_endpoints_require_session_token_except_health_and_preflight() {
    let security = test_security();
    let protected = [
        (Method::GET, "/events"),
        (Method::GET, "/config"),
        (Method::GET, "/inspect/topology"),
        (Method::GET, "/inspect/topology.mmd"),
        (Method::GET, "/inspect/topology.map"),
        (Method::GET, "/inspect/topology.runtime"),
        (Method::GET, "/inspect/topology.runtime.mmd"),
        (Method::GET, "/sessions"),
        (Method::GET, "/sessions/current"),
        (Method::GET, "/history"),
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
    let security = HttpSecurity::from_config(&config);

    assert!(!config.require_session_token);
    assert!(!request_requires_session_token(
        &Method::GET,
        "/config",
        &security
    ));
}

#[test]
fn token_auth_accepts_header_authorization_and_query_tokens() {
    let security = test_security();
    let header_request = Request::builder()
        .uri("/config")
        .header("x-proteus-session", "session-secret")
        .body(())
        .expect("request");
    let legacy_header_request = Request::builder()
        .uri("/config")
        .header("x-proteus-session-token", "session-secret")
        .body(())
        .expect("request");
    let bearer_request = Request::builder()
        .uri("/config")
        .header(AUTHORIZATION, "Bearer session-secret")
        .body(())
        .expect("request");
    let query_request = Request::builder()
        .uri("/events?token=session-secret")
        .body(())
        .expect("request");
    let alias_query_request = Request::builder()
        .uri("/events?session_token=session-secret")
        .body(())
        .expect("request");
    let web_query_request = Request::builder()
        .uri("/events?session=session-secret")
        .body(())
        .expect("request");

    assert!(request_has_valid_token(&header_request, &security));
    assert!(request_has_valid_token(&legacy_header_request, &security));
    assert!(request_has_valid_token(&bearer_request, &security));
    assert!(request_has_valid_token(&query_request, &security));
    assert!(request_has_valid_token(&alias_query_request, &security));
    assert!(request_has_valid_token(&web_query_request, &security));
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
    let invalid_header = Request::builder()
        .uri("/config")
        .header("x-proteus-session", "wrong")
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
    assert!(!request_has_valid_token(&invalid_header, &security));
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
        Some("authorization, content-type, x-proteus-session, x-proteus-session-token")
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
        .header("x-proteus-session", "session-secret")
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
        .header("x-proteus-session", "session-secret")
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

#[tokio::test]
async fn route_inspect_topology_returns_json_and_mermaid() {
    let (state, server) = test_state().await;

    let response = route_request(state.clone(), authed_get_request("/inspect/topology"))
        .await
        .expect("topology response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let body = response_bytes(response).await;
    let topology: Value = serde_json::from_slice(&body).expect("topology JSON");
    assert_eq!(
        topology.pointer("/profile").and_then(Value::as_str),
        Some("dev-basic")
    );
    assert!(
        topology
            .pointer("/slots")
            .and_then(Value::as_array)
            .is_some_and(|slots| slots
                .iter()
                .any(|slot| { slot.get("id").and_then(Value::as_str) == Some("workflow") }))
    );
    assert!(
        topology
            .pointer("/tools")
            .and_then(Value::as_array)
            .is_some()
    );
    let edges = topology
        .pointer("/edges")
        .and_then(Value::as_array)
        .expect("topology edges");
    assert!(edges.iter().any(|edge| {
        edge.get("kind").and_then(Value::as_str) == Some("selects")
            || edge.get("kind").and_then(Value::as_str) == Some("runtime")
    }));

    let response = route_request(state.clone(), authed_get_request("/inspect/topology.mmd"))
        .await
        .expect("mermaid response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let body = String::from_utf8(response_bytes(response).await.to_vec()).expect("utf8");
    assert!(body.starts_with("flowchart LR"));
    assert!(body.contains("Turn pipeline"));
    assert!(body.contains("workflow<br/>"));
    assert!(body.contains("Backends / post-turn"));
    assert!(body.contains("memory_policy"));
    assert!(body.contains("selects modules"));
    assert!(!body.contains("Warnings"));

    let response = route_request(state.clone(), authed_get_request("/inspect/topology.map"))
        .await
        .expect("map response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let body = String::from_utf8(response_bytes(response).await.to_vec()).expect("utf8");
    assert!(body.starts_with("Proteus topology map"));
    assert!(body.contains("Slot/module map"));

    let response = route_request(
        state.clone(),
        authed_get_request("/inspect/topology.runtime"),
    )
    .await
    .expect("runtime response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let body = String::from_utf8(response_bytes(response).await.to_vec()).expect("utf8");
    assert!(body.starts_with("Proteus runtime path"));
    assert!(body.contains("Active product path"));
    assert!(body.contains("tools"));
    assert!(!body.contains("Plugin contribution map"));

    let response = route_request(state, authed_get_request("/inspect/topology.runtime.mmd"))
        .await
        .expect("runtime mermaid response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let body = String::from_utf8(response_bytes(response).await.to_vec()).expect("utf8");
    assert!(body.starts_with("flowchart LR"));
    assert!(body.contains("ToolRegistry"));
    assert!(body.contains("Final output"));
    assert!(!body.contains("Plugin contribution map"));
    server.shutdown().await;
}

#[tokio::test]
async fn event_stream_flushes_initial_heartbeat() {
    let (state, server) = test_state().await;
    let request = Request::builder()
        .method(Method::GET)
        .uri("/events?token=session-secret")
        .header(ORIGIN, "http://127.0.0.1:1420")
        .body(empty_body())
        .expect("request");

    let response = route_request(state, request).await.expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );

    let mut body = response.into_body();
    let frame = tokio::time::timeout(Duration::from_secs(1), body.frame())
        .await
        .expect("SSE should flush a first frame")
        .expect("SSE body should stay open")
        .expect("SSE frame should be valid");
    assert_eq!(
        frame.data_ref().expect("heartbeat should be data"),
        &Bytes::from_static(b": connected\n\n")
    );
    drop(body);
    server.shutdown().await;
}

#[tokio::test]
async fn request_dispatch_sets_permission_mode() {
    let cwd = tempfile::tempdir().expect("cwd");
    let server = AgentAppServer::launch(AppConfig::default(), cwd.path().to_path_buf(), None)
        .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let output = execute_app_request(
        &state,
        StdioRequest::SetPermissionMode {
            id: Some("mode-1".to_owned()),
            mode: PermissionMode::Auto,
        },
    )
    .await;

    match output {
        StdioOutput::Response {
            id,
            ok,
            output,
            error,
        } => {
            assert_eq!(id.as_deref(), Some("mode-1"));
            assert!(ok);
            assert_eq!(
                output
                    .as_ref()
                    .and_then(|value| value.get("mode"))
                    .and_then(Value::as_str),
                Some("auto")
            );
            assert!(error.is_none());
        }
        StdioOutput::Event { .. } => panic!("expected command response"),
        _ => panic!("unexpected output variant"),
    }
    assert_eq!(server.permission_mode().await, PermissionMode::Auto);
    server.shutdown().await;
}

#[tokio::test]
async fn request_dispatch_sets_reasoning_effort() {
    let cwd = tempfile::tempdir().expect("cwd");
    let server = AgentAppServer::launch(AppConfig::default(), cwd.path().to_path_buf(), None)
        .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let output = execute_app_request(
        &state,
        StdioRequest::SetReasoningEffort {
            id: Some("effort-1".to_owned()),
            effort: Some("high".to_owned()),
        },
    )
    .await;

    match output {
        StdioOutput::Response {
            id,
            ok,
            output,
            error,
        } => {
            assert_eq!(id.as_deref(), Some("effort-1"));
            assert!(ok);
            assert_eq!(
                output
                    .as_ref()
                    .and_then(|value| value.get("effort"))
                    .and_then(Value::as_str),
                Some("high")
            );
            assert!(error.is_none());
        }
        StdioOutput::Event { .. } => panic!("expected command response"),
        _ => panic!("unexpected output variant"),
    }
    let summary = server.config_summary().await;
    assert_eq!(
        summary.pointer("/reasoning/effort").and_then(Value::as_str),
        Some("high")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn request_dispatch_sets_model_and_reasoning_enabled() {
    let cwd = tempfile::tempdir().expect("cwd");
    let server = AgentAppServer::launch(AppConfig::default(), cwd.path().to_path_buf(), None)
        .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let model_output = execute_app_request(
        &state,
        StdioRequest::SetModel {
            id: Some("model-1".to_owned()),
            model: "deepseek-v4-pro".to_owned(),
        },
    )
    .await;
    assert!(matches!(
        model_output,
        StdioOutput::Response { ok: true, .. }
    ));

    let reasoning_output = execute_app_request(
        &state,
        StdioRequest::SetReasoningEnabled {
            id: Some("reasoning-1".to_owned()),
            enabled: false,
        },
    )
    .await;
    assert!(matches!(
        reasoning_output,
        StdioOutput::Response { ok: true, .. }
    ));

    let summary = server.config_summary().await;
    assert_eq!(
        summary.pointer("/model/name").and_then(Value::as_str),
        Some("deepseek-v4-pro")
    );
    assert_eq!(
        summary
            .pointer("/reasoning/effort_options")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["high", "max"])
    );
    assert_eq!(
        summary
            .pointer("/reasoning/enabled")
            .and_then(Value::as_bool),
        Some(false)
    );
    server.shutdown().await;
}

#[tokio::test]
async fn route_approval_resolves_pending_request_with_auth_and_cors() {
    let (state, server) = test_state().await;
    let (approval_tx, approval_rx) = tokio::sync::oneshot::channel();
    let approval_id = "approval-route".to_owned();
    server.pending_approvals.lock().await.insert(
        approval_id.clone(),
        pending_approval_entry(&approval_id, approval_tx),
    );
    let request = Request::builder()
        .method(Method::POST)
        .uri("/approval")
        .header(ORIGIN, "http://127.0.0.1:1420")
        .header("x-proteus-session", "session-secret")
        .header(CONTENT_TYPE, "application/json")
        .body(json_body(json!({
            "id": "approval-1",
            "approval_id": approval_id,
            "approved": true,
            "note": "route approval",
            "cache": "exact_call",
        })))
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
    match response_output(response).await {
        StdioOutput::Response { id, ok, error, .. } => {
            assert_eq!(id.as_deref(), Some("approval-1"));
            assert!(ok, "approval response should succeed: {error:?}");
        }
        other => panic!("expected response output, got {other:?}"),
    }

    let approval = approval_rx.await.expect("approval should resolve");
    assert!(approval.approved);
    assert_eq!(approval.note.as_deref(), Some("route approval"));
    assert_eq!(approval.cache, ApprovalCacheScope::ExactCall);
    assert!(server.pending_approvals.lock().await.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn route_user_input_resolves_pending_request_with_auth_and_cors() {
    let (state, server) = test_state().await;
    let (input_tx, input_rx) = tokio::sync::oneshot::channel();
    let request_id = "input-route".to_owned();
    server.pending_user_inputs.lock().await.insert(
        request_id.clone(),
        pending_user_input_entry(&request_id, input_tx),
    );
    let request = Request::builder()
        .method(Method::POST)
        .uri("/user-input")
        .header(ORIGIN, "http://127.0.0.1:1420")
        .header("x-proteus-session", "session-secret")
        .header(CONTENT_TYPE, "application/json")
        .body(json_body(json!({
            "id": "input-1",
            "request_id": request_id,
            "response": {
                "answers": {
                    "scope": {
                        "answers": ["small"]
                    }
                }
            }
        })))
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
    match response_output(response).await {
        StdioOutput::Response { id, ok, error, .. } => {
            assert_eq!(id.as_deref(), Some("input-1"));
            assert!(ok, "user-input response should succeed: {error:?}");
        }
        other => panic!("expected response output, got {other:?}"),
    }

    let response = input_rx.await.expect("user input should resolve");
    assert_eq!(
        response.answers,
        HashMap::from([(
            "scope".to_owned(),
            UserInputAnswer::new(vec!["small".to_owned()])
        )])
    );
    assert!(server.pending_user_inputs.lock().await.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn route_pending_returns_current_pending_requests_with_auth_and_cors() {
    let (state, server) = test_state().await;
    let (approval_tx, _approval_rx) = tokio::sync::oneshot::channel();
    let approval_id = "approval-pending".to_owned();
    server.pending_approvals.lock().await.insert(
        approval_id.clone(),
        pending_approval_entry(&approval_id, approval_tx),
    );
    let (input_tx, _input_rx) = tokio::sync::oneshot::channel();
    let request_id = "input-pending".to_owned();
    server.pending_user_inputs.lock().await.insert(
        request_id.clone(),
        pending_user_input_entry(&request_id, input_tx),
    );

    let response = route_request(state, authed_get_request("/pending"))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("http://127.0.0.1:1420")
    );
    let bytes = response_bytes(response).await;
    let pending: crate::app_server::AppPendingRequests =
        serde_json::from_slice(&bytes).expect("pending JSON");
    assert_eq!(pending.approvals.len(), 1);
    assert_eq!(pending.approvals[0].approval_id, approval_id);
    assert_eq!(pending.user_inputs.len(), 1);
    assert_eq!(pending.user_inputs[0].request_id, request_id);
    server.shutdown().await;
}

#[tokio::test]
async fn route_history_can_read_requested_session_without_switching_current() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let saved_session_id = crate::domain::new_session_id();
    let saved_store =
        crate::core::SessionStore::new(config_dir.path(), cwd.path(), saved_session_id);
    saved_store
        .append_messages(&[crate::model_standard::CanonicalMessage::text(
            crate::model_standard::MessageRole::User,
            "saved cold history",
        )])
        .await
        .expect("append saved history");
    let server = AgentAppServer::launch(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

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
    assert_ne!(
        state.current_server().await.session_dir_path().as_deref(),
        Some(saved_store.session_dir())
    );

    server.shutdown().await;
}

#[tokio::test]
async fn route_new_session_replaces_active_session_dir() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .expect("app server");
    let original_session_dir = server
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("original session dir")
        .to_owned();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");

    assert_eq!(response.status(), StatusCode::OK);
    let output = response_output(response).await;
    let StdioOutput::Response {
        ok: true,
        output: Some(summary),
        ..
    } = output
    else {
        panic!("expected successful new-session response");
    };
    let next_session_dir = summary
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("new session dir");
    assert_ne!(next_session_dir, original_session_dir);
    assert_eq!(
        state
            .current_server()
            .await
            .config_summary()
            .await
            .get("session_dir")
            .and_then(Value::as_str),
        Some(next_session_dir)
    );

    let response = route_request(state.clone(), authed_get_request("/sessions/current"))
        .await
        .expect("sessions response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response_bytes(response).await;
    let sessions: Vec<Value> = serde_json::from_slice(&bytes).expect("sessions JSON");
    assert!(!PathBuf::from(next_session_dir).exists());
    assert!(
        sessions.iter().any(|session| {
            session.get("session_dir").and_then(Value::as_str) == Some(next_session_dir)
                && session.get("message_count").and_then(Value::as_u64) == Some(0)
        }),
        "active empty session should be listed before the first message"
    );

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/resume",
            json!({
                "id": "resume-original",
                "session_dir": original_session_dir,
            }),
        ),
    )
    .await
    .expect("resume response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = route_request(state.clone(), authed_get_request("/sessions/current"))
        .await
        .expect("sessions response after resume");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response_bytes(response).await;
    let sessions: Vec<Value> = serde_json::from_slice(&bytes).expect("sessions JSON");
    assert!(
        !sessions
            .iter()
            .any(|session| session.get("session_dir").and_then(Value::as_str)
                == Some(next_session_dir)),
        "background empty idle session should disappear after switching away"
    );

    server.shutdown().await;
    state.current_server().await.shutdown().await;
}

#[tokio::test]
async fn route_new_session_keeps_background_turn_registered() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .expect("app server");
    let original_session_dir = server
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("original session dir")
        .to_owned();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());
    let cancellation = CancellationToken::new();
    state.running_turns.lock().await.insert(
        "turn-background".to_owned(),
        RunningTurn::new(
            cancellation.clone(),
            Some(PathBuf::from(&original_session_dir)),
        ),
    );

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state
            .running_turns
            .lock()
            .await
            .contains_key("turn-background")
    );
    let original_session_path = PathBuf::from(&original_session_dir);
    assert!(
        state
            .server_for_session_dir(&original_session_path)
            .await
            .is_some()
    );

    let response = route_request(state.clone(), authed_get_request("/sessions/current"))
        .await
        .expect("sessions response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response_bytes(response).await;
    let sessions: Vec<Value> = serde_json::from_slice(&bytes).expect("sessions JSON");
    let background = sessions
        .iter()
        .find(|session| {
            session.get("session_dir").and_then(Value::as_str) == Some(&original_session_dir)
        })
        .expect("running background session should be listed");
    assert_eq!(
        background
            .pointer("/activity/status")
            .and_then(Value::as_str),
        Some("running")
    );

    cancellation.cancel();
    server.shutdown().await;
    state.current_server().await.shutdown().await;
}

#[tokio::test]
async fn route_send_async_targets_requested_session_after_current_switches() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .expect("app server");
    let original_session_dir = server
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("original session dir")
        .to_owned();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");
    assert_eq!(response.status(), StatusCode::OK);
    let current_after_switch = state.current_server().await;
    assert!(!current_after_switch.is_session_dir(PathBuf::from(&original_session_dir).as_path()));

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/send-async",
            json!({
                "id": "send-old-session",
                "text": "sent to original session",
                "session_dir": original_session_dir,
            }),
        ),
    )
    .await
    .expect("send response");
    assert_eq!(response.status(), StatusCode::OK);
    match response_output(response).await {
        StdioOutput::Response { ok: true, .. } => {}
        other => panic!("expected successful send response, got {other:?}"),
    }

    let original_transcript = wait_for_transcript_text(&server, "sent to original session").await;
    assert!(
        original_transcript
            .iter()
            .any(|message| message.role == "user" && message.text == "sent to original session")
    );
    assert!(
        !state
            .current_server()
            .await
            .transcript()
            .await
            .iter()
            .any(|message| message.text == "sent to original session")
    );

    server.shutdown().await;
    state.current_server().await.shutdown().await;
}

#[tokio::test]
async fn route_resume_reuses_live_session_without_materialized_metadata() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .expect("app server");
    let original_session_dir = server
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("original session dir")
        .to_owned();
    assert!(!PathBuf::from(&original_session_dir).exists());
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/resume",
            json!({
                "id": "resume-original",
                "session_dir": original_session_dir.clone(),
            }),
        ),
    )
    .await
    .expect("resume response");

    assert_eq!(response.status(), StatusCode::OK);
    let output = response_output(response).await;
    let StdioOutput::Response {
        ok: true,
        output: Some(summary),
        ..
    } = output
    else {
        panic!("expected successful resume response");
    };
    assert_eq!(
        summary.get("session_dir").and_then(Value::as_str),
        Some(original_session_dir.as_str())
    );
    assert_eq!(
        state
            .current_server()
            .await
            .config_summary()
            .await
            .get("session_dir")
            .and_then(Value::as_str),
        Some(original_session_dir.as_str())
    );

    state.current_server().await.shutdown().await;
}

#[tokio::test]
async fn route_approval_resolves_background_session_request() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());
    let (responder, response_rx) = tokio::sync::oneshot::channel();
    let approval_id = "approval-background".to_owned();
    server.pending_approvals.lock().await.insert(
        approval_id.clone(),
        pending_approval_entry(&approval_id, responder),
    );

    let response = route_request(
        state.clone(),
        authed_json_request("/new-session", json!({ "id": "new-session" })),
    )
    .await
    .expect("new session response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/approval",
            json!({
                "id": "approval-response",
                "approval_id": approval_id,
                "approved": true,
                "note": "approved in background",
                "cache": "none",
            }),
        ),
    )
    .await
    .expect("approval response");

    assert_eq!(response.status(), StatusCode::OK);
    match response_output(response).await {
        StdioOutput::Response { ok, error, .. } => {
            assert!(ok, "approval response should succeed: {error:?}");
        }
        other => panic!("expected response output, got {other:?}"),
    }
    let approval = response_rx.await.expect("approval should resolve");
    assert!(approval.approved);
    assert_eq!(approval.note.as_deref(), Some("approved in background"));

    server.shutdown().await;
    state.current_server().await.shutdown().await;
}

#[tokio::test]
async fn route_delete_unsaved_active_session_opens_new_one() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .expect("app server");
    let original_session_dir = server
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("original session dir")
        .to_owned();
    server.start_session().await.expect("start session");
    assert!(!PathBuf::from(&original_session_dir).exists());
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/delete-session",
            json!({
                "id": "delete-session",
                "session_dir": original_session_dir,
            }),
        ),
    )
    .await
    .expect("delete session response");

    assert_eq!(response.status(), StatusCode::OK);
    let output = response_output(response).await;
    let StdioOutput::Response {
        ok: true,
        output: Some(summary),
        ..
    } = output
    else {
        panic!("expected successful delete-session response");
    };
    assert_eq!(summary.get("deleted").and_then(Value::as_bool), Some(false));
    assert_eq!(
        summary.get("active_replaced").and_then(Value::as_bool),
        Some(true)
    );
    assert!(!PathBuf::from(&original_session_dir).exists());
    let next_session_dir = state
        .current_server()
        .await
        .config_summary()
        .await
        .get("session_dir")
        .and_then(Value::as_str)
        .expect("next session dir")
        .to_owned();
    assert_ne!(next_session_dir, original_session_dir);
    assert!(!PathBuf::from(next_session_dir).exists());

    server.shutdown().await;
    state.current_server().await.shutdown().await;
}

#[tokio::test]
async fn route_send_async_acknowledges_while_turn_keeps_running() {
    let (state, server) = dogfood_loop_state().await;
    let mut event_rx = server.subscribe();
    let turn_id = "turn-async".to_owned();

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/send-async",
            json!({
                "id": turn_id,
                "text": "apply_patch",
            }),
        ),
    )
    .await
    .expect("send-async response");

    assert_eq!(response.status(), StatusCode::OK);
    let output = response_output(response).await;
    let StdioOutput::Response {
        ok: true,
        output: Some(summary),
        ..
    } = output
    else {
        panic!("expected successful send-async response");
    };
    assert_eq!(summary.get("accepted").and_then(Value::as_bool), Some(true));
    assert_eq!(
        summary.get("turn_id").and_then(Value::as_str),
        Some(turn_id.as_str())
    );

    let approval = wait_for_approval_request(&mut event_rx).await;
    assert_eq!(approval.call.name, "apply_patch");
    assert!(state.running_turns.lock().await.contains_key(&turn_id));

    let response = route_request(
        state.clone(),
        authed_json_request(
            "/cancel",
            json!({
                "id": "cancel-async",
                "target_id": turn_id,
            }),
        ),
    )
    .await
    .expect("cancel response");
    assert_eq!(response.status(), StatusCode::OK);
    let output = response_output(response).await;
    assert!(matches!(output, StdioOutput::Response { ok: true, .. }));
    assert!(state.running_turns.lock().await.is_empty());

    server.shutdown().await;
}

#[tokio::test]
async fn route_send_approval_loop_completes_after_http_approval() {
    let (state, server) = dogfood_loop_state().await;
    let mut event_rx = server.subscribe();
    let send_state = state.clone();
    let send_task = tokio::spawn(async move {
        let request = authed_json_request(
            "/send",
            json!({
                "id": "turn-approval",
                "text": "apply_patch",
            }),
        );
        route_request(send_state, request)
            .await
            .expect("send response")
    });

    let approval = wait_for_approval_request(&mut event_rx).await;
    assert_eq!(approval.call.name, "apply_patch");
    assert_eq!(
        approval.tool_spec.as_ref().map(|spec| spec.name.as_str()),
        Some("apply_patch")
    );
    let preview = approval.preview.as_ref().expect("approval preview");
    assert_eq!(preview.kind, "patch");
    assert_eq!(preview.language.as_deref(), Some("diff"));
    assert!(
        preview
            .body
            .as_deref()
            .is_some_and(|body| body.contains("*** Begin Patch"))
    );

    let approval_response = route_request(
        state.clone(),
        authed_json_request(
            "/approval",
            json!({
                "id": "approval-response",
                "approval_id": approval.approval_id,
                "approved": true,
                "note": "approved by route loop test",
                "cache": "exact_call",
            }),
        ),
    )
    .await
    .expect("approval response");
    assert_eq!(approval_response.status(), StatusCode::OK);
    match response_output(approval_response).await {
        StdioOutput::Response { id, ok, error, .. } => {
            assert_eq!(id.as_deref(), Some("approval-response"));
            assert!(ok, "approval response should succeed: {error:?}");
        }
        other => panic!("expected approval response output, got {other:?}"),
    }

    let send_response = tokio::time::timeout(Duration::from_secs(2), send_task)
        .await
        .expect("send should finish after approval")
        .expect("send task should join");
    assert_eq!(send_response.status(), StatusCode::OK);
    match response_output(send_response).await {
        StdioOutput::Response {
            id,
            ok,
            output,
            error,
        } => {
            assert_eq!(id.as_deref(), Some("turn-approval"));
            assert!(ok, "send should succeed after approval: {error:?}");
            let text = output
                .as_ref()
                .and_then(|value| value.get("text"))
                .and_then(Value::as_str)
                .expect("send output text");
            assert!(text.contains("Fake final answer after tool result"));
            assert!(text.contains("patch applier is disabled"));
        }
        other => panic!("expected send response output, got {other:?}"),
    }
    assert!(server.pending_approvals.lock().await.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn route_send_user_input_loop_completes_after_http_response() {
    let (state, server) = dogfood_loop_state().await;
    let mut event_rx = server.subscribe();
    let send_state = state.clone();
    let send_task = tokio::spawn(async move {
        let request = authed_json_request(
            "/send",
            json!({
                "id": "turn-input",
                "text": "request_user_input",
            }),
        );
        route_request(send_state, request)
            .await
            .expect("send response")
    });

    let input = wait_for_user_input_request(&mut event_rx).await;
    assert_eq!(input.questions.len(), 1);
    assert_eq!(
        input.questions[0].question,
        "Which smoke path should continue?"
    );

    let input_response = route_request(
        state.clone(),
        authed_json_request(
            "/user-input",
            json!({
                "id": "input-response",
                "request_id": input.request_id,
                "response": {
                    "answers": {
                        "Choice": {
                            "answers": ["Approve"]
                        }
                    }
                }
            }),
        ),
    )
    .await
    .expect("user-input response");
    assert_eq!(input_response.status(), StatusCode::OK);
    match response_output(input_response).await {
        StdioOutput::Response { id, ok, error, .. } => {
            assert_eq!(id.as_deref(), Some("input-response"));
            assert!(ok, "user-input response should succeed: {error:?}");
        }
        other => panic!("expected user-input response output, got {other:?}"),
    }

    let send_response = tokio::time::timeout(Duration::from_secs(2), send_task)
        .await
        .expect("send should finish after user input")
        .expect("send task should join");
    assert_eq!(send_response.status(), StatusCode::OK);
    match response_output(send_response).await {
        StdioOutput::Response {
            id,
            ok,
            output,
            error,
        } => {
            assert_eq!(id.as_deref(), Some("turn-input"));
            assert!(ok, "send should succeed after user input: {error:?}");
            let text = output
                .as_ref()
                .and_then(|value| value.get("text"))
                .and_then(Value::as_str)
                .expect("send output text");
            assert!(text.contains("Fake final answer after tool result"));
            assert!(text.contains("User answered:"));
            assert!(text.contains("Choice: Approve"));
        }
        other => panic!("expected send response output, got {other:?}"),
    }
    assert!(server.pending_user_inputs.lock().await.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn cancel_unknown_turn_returns_protocol_error() {
    let cwd = tempfile::tempdir().expect("cwd");
    let server = AgentAppServer::launch(AppConfig::default(), cwd.path().to_path_buf(), None)
        .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());

    let output = execute_app_request(
        &state,
        StdioRequest::Cancel {
            id: Some("cancel-1".to_owned()),
            target_id: "missing".to_owned(),
        },
    )
    .await;

    match output {
        StdioOutput::Response { ok, error, .. } => {
            assert!(!ok);
            assert_eq!(
                error.as_deref(),
                Some("unknown or completed turn id: missing")
            );
        }
        StdioOutput::Event { .. } => panic!("expected command response"),
        _ => panic!("unexpected output variant"),
    }
    server.shutdown().await;
}

#[tokio::test]
async fn cancel_active_turn_clears_pending_approval_and_user_input() {
    let cwd = tempfile::tempdir().expect("cwd");
    let server = AgentAppServer::launch(AppConfig::default(), cwd.path().to_path_buf(), None)
        .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security());
    let turn_id = "turn-cancel".to_owned();
    let cancellation = CancellationToken::new();
    state.running_turns.lock().await.insert(
        turn_id.clone(),
        RunningTurn::new(cancellation.clone(), server.session_dir_path()),
    );

    let (approval_tx, approval_rx) = tokio::sync::oneshot::channel();
    let approval_id = "approval-cancel".to_owned();
    server.pending_approvals.lock().await.insert(
        approval_id.clone(),
        pending_approval_entry(&approval_id, approval_tx),
    );

    let (input_tx, input_rx) = tokio::sync::oneshot::channel();
    let request_id = "input-cancel".to_owned();
    server.pending_user_inputs.lock().await.insert(
        request_id.clone(),
        pending_user_input_entry(&request_id, input_tx),
    );

    let output = execute_app_request(
        &state,
        StdioRequest::Cancel {
            id: Some("cancel-1".to_owned()),
            target_id: turn_id,
        },
    )
    .await;

    match output {
        StdioOutput::Response { ok, error, .. } => {
            assert!(ok, "cancel should succeed: {error:?}");
        }
        StdioOutput::Event { .. } => panic!("expected command response"),
        _ => panic!("unexpected output variant"),
    }

    assert!(cancellation.is_cancelled());
    assert!(state.running_turns.lock().await.is_empty());
    assert!(server.pending_approvals.lock().await.is_empty());
    assert!(server.pending_user_inputs.lock().await.is_empty());

    let approval = approval_rx.await.expect("approval should be resolved");
    assert!(!approval.approved);
    assert_eq!(approval.note.as_deref(), Some("turn canceled by client"));
    let input = input_rx.await.expect("user input should be resolved");
    assert!(input.answers.is_empty());

    server.shutdown().await;
}

#[test]
fn sse_output_wraps_protocol_output_as_json_data() {
    let bytes = encode_sse_output(&StdioOutput::Response {
        id: Some("req-1".to_owned()),
        ok: true,
        output: None,
        error: None,
    });
    let text = std::str::from_utf8(&bytes).expect("utf8");

    assert!(text.starts_with("event: output\ndata: "));
    assert!(text.contains(r#""type":"response""#));
    assert!(text.contains(r#""id":"req-1""#));
    assert!(text.ends_with("\n\n"));
}
