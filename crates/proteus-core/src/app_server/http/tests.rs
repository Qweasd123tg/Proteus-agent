use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use hyper::header::{AUTHORIZATION, ORIGIN};
use proteus_contracts::contracts::ApprovalCacheScope;
use serde_json::Value;

use super::*;
use crate::contracts::{
    ApprovalResponse, CancellationToken, UserInputAnswer,
    UserInputRequest as ContractUserInputRequest, UserInputResponse,
};
use crate::core::{AppConfig, ModuleCatalog};
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

async fn test_state() -> (HttpAppState, AppServerHandle, tempfile::TempDir) {
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch(
        crate::test_model::config(),
        config_dir.path().to_path_buf(),
        Some(&config_path),
    )
    .await
    .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;
    (state, server, config_dir)
}

async fn register_pending_approval(
    server: &AppServerHandle,
    approval_id: &str,
    responder: tokio::sync::oneshot::Sender<ApprovalResponse>,
) {
    crate::app_server::approvals::register_pending_approval(
        &server.pending_approvals,
        &server.events,
        crate::app_server::AppApprovalRequest::new(
            approval_id.to_owned(),
            ToolCall::new(new_call_id(), "write_file", json!({ "path": "notes.txt" })),
            PathBuf::from("/workspace"),
            "test approval".to_owned(),
            None,
        ),
        responder,
    )
    .await;
}

async fn register_pending_user_input(
    server: &AppServerHandle,
    request_id: &str,
    responder: tokio::sync::oneshot::Sender<UserInputResponse>,
) {
    crate::app_server::user_inputs::register_pending_user_input(
        &server.pending_user_inputs,
        &server.events,
        ContractUserInputRequest::new(
            request_id.to_owned(),
            PathBuf::from("/workspace"),
            Vec::new(),
        ),
        responder,
    )
    .await;
}

async fn dogfood_loop_state() -> (HttpAppState, AppServerHandle, tempfile::TempDir) {
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let server = AgentAppServer::launch_with_module_catalog(
        dogfood_loop_config(),
        config_dir.path().to_path_buf(),
        Some(&config_path),
        dogfood_loop_catalog(),
    )
    .await
    .expect("app server");
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;
    (state, server, config_dir)
}

fn dogfood_loop_config() -> AppConfig {
    let mut config = crate::test_model::config();
    let active_provider = config.active_provider.clone();
    config
        .providers
        .get_mut(&active_provider)
        .expect("default provider")
        .stream = false;
    crate::test_support::select_test_modules(&mut config, "coding.single_loop");
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

fn dogfood_loop_catalog() -> ModuleCatalog {
    crate::test_support::module_catalog()
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
        .header(AUTHORIZATION, "Bearer session-secret")
        .body(empty_body())
        .expect("request")
}

fn authed_json_request(path: &str, value: Value) -> Request<Full<Bytes>> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(ORIGIN, "http://127.0.0.1:1420")
        .header(AUTHORIZATION, "Bearer session-secret")
        .header(CONTENT_TYPE, "application/json")
        .body(json_body(value))
        .expect("request")
}

fn session_uri(path: &str, server: &AppServerHandle) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };
    format!(
        "{path}{separator}session_dir={}",
        server
            .session_dir_path()
            .expect("test server must have a session directory")
            .display()
    )
}

fn session_query(server: &AppServerHandle) -> String {
    format!(
        "session_dir={}",
        server
            .session_dir_path()
            .expect("test server must have a session directory")
            .display()
    )
}

async fn shutdown_test_servers(state: &HttpAppState) {
    for server in state.all_servers().await {
        server.shutdown().await;
    }
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
        let transcript = server.transcript().await.expect("transcript");
        if transcript.iter().any(|message| message.text == text) {
            return transcript;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    server.transcript().await.expect("transcript")
}

mod addressing;
mod commands;
mod config;
mod inspection;
mod live;
mod pending;
mod security;
mod session_reads;
mod sessions;
mod sse;
mod turns;

mod workspace;
