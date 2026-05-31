use std::{
    collections::HashMap,
    convert::Infallible,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Result, anyhow};
use async_stream::stream;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Frame, Incoming},
    header::{CACHE_CONTROL, CONNECTION, CONTENT_TYPE},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use proteus_contracts::contracts::{ApprovalCacheScope, UserInputResponse};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{Mutex, broadcast},
};

use crate::{
    contracts::CancellationToken,
    core::AppConfig,
    domain::{AgentOutput, PermissionMode},
};

use super::{
    AgentAppServer, AppServerEvent, AppServerHandle,
    protocol::{StdioOutput, StdioRequest},
};

type HttpBody = UnsyncBoxBody<Bytes, Infallible>;
type HttpResponse = Response<HttpBody>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpServerConfig {
    pub bind: SocketAddr,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787),
        }
    }
}

#[derive(Clone)]
struct HttpAppState {
    server: Arc<Mutex<AppServerHandle>>,
    running_turns: Arc<Mutex<HashMap<String, CancellationToken>>>,
    shutdown: broadcast::Sender<()>,
}

impl HttpAppState {
    fn new(server: AppServerHandle, shutdown: broadcast::Sender<()>) -> Self {
        Self {
            server: Arc::new(Mutex::new(server)),
            running_turns: Arc::new(Mutex::new(HashMap::new())),
            shutdown,
        }
    }

    async fn current_server(&self) -> AppServerHandle {
        self.server.lock().await.clone()
    }
}

#[derive(Debug, Deserialize)]
struct SendRequest {
    id: Option<String>,
    text: String,
}

#[derive(Debug, Deserialize)]
struct ApprovalRequest {
    id: Option<String>,
    approval_id: String,
    approved: bool,
    note: Option<String>,
    #[serde(default)]
    cache: ApprovalCacheScope,
}

#[derive(Debug, Deserialize)]
struct UserInputRequest {
    id: Option<String>,
    request_id: String,
    response: UserInputResponse,
}

#[derive(Debug, Deserialize)]
struct CancelRequest {
    id: Option<String>,
    target_id: String,
}

#[derive(Debug, Deserialize)]
struct SetPermissionModeRequest {
    id: Option<String>,
    mode: PermissionMode,
}

#[derive(Debug, Deserialize)]
struct SetModelRequest {
    id: Option<String>,
    model: String,
}

#[derive(Debug, Deserialize)]
struct SetReasoningEffortRequest {
    id: Option<String>,
    effort: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetReasoningEnabledRequest {
    id: Option<String>,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct ResumeSessionRequest {
    id: Option<String>,
    session_dir: PathBuf,
}

pub async fn run_http_app_server(
    config: AppConfig,
    cwd: PathBuf,
    config_path: Option<PathBuf>,
    resume_session_dir: Option<PathBuf>,
    http_config: HttpServerConfig,
) -> Result<()> {
    let server = if let Some(session_dir) = resume_session_dir {
        AgentAppServer::launch_resumed(config, cwd, config_path.as_deref(), session_dir)?
    } else {
        AgentAppServer::launch(config, cwd, config_path.as_deref())?
    };
    let (shutdown, mut shutdown_rx) = broadcast::channel(1);
    let state = HttpAppState::new(server, shutdown);
    let listener = TcpListener::bind(http_config.bind).await?;
    println!(
        "Proteus app-server HTTP listening on http://{}",
        listener.local_addr()?
    );
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let state = state.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let service = service_fn(move |request| route_request(state.clone(), request));
                    if let Err(error) = http1::Builder::new().serve_connection(io, service).await {
                        eprintln!("app-server HTTP connection error: {error}");
                    }
                });
            }
            _ = shutdown_rx.recv() => break,
        }
    }
    Ok(())
}

async fn route_request(
    state: HttpAppState,
    request: Request<Incoming>,
) -> Result<HttpResponse, Infallible> {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    let response = match (method, path.as_str()) {
        (Method::OPTIONS, _) => empty_response(StatusCode::NO_CONTENT),
        (Method::GET, "/health") => json_response(StatusCode::OK, &json!({ "ok": true })),
        (Method::GET, "/events") => sse_response(state).await,
        (Method::GET, "/config") => {
            let summary = state.current_server().await.config_summary().await;
            json_response(StatusCode::OK, &summary)
        }
        (Method::GET, "/sessions") => match state.current_server().await.session_summaries() {
            Ok(sessions) => json_response(StatusCode::OK, &sessions),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error:#}")),
        },
        (Method::GET, "/history") => {
            let transcript = state.current_server().await.transcript().await;
            json_response(StatusCode::OK, &transcript)
        }
        (Method::POST, "/request") => match read_json::<StdioRequest>(request).await {
            Ok(command) => {
                json_response(StatusCode::OK, &execute_app_request(&state, command).await)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/send") => match read_json::<SendRequest>(request).await {
            Ok(command) => {
                let output = execute_app_request(
                    &state,
                    StdioRequest::Send {
                        id: command.id,
                        text: command.text,
                    },
                )
                .await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/approval") => match read_json::<ApprovalRequest>(request).await {
            Ok(command) => {
                let output = execute_app_request(
                    &state,
                    StdioRequest::Approval {
                        id: command.id,
                        approval_id: command.approval_id,
                        approved: command.approved,
                        note: command.note,
                        cache: command.cache,
                    },
                )
                .await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/user-input") => match read_json::<UserInputRequest>(request).await {
            Ok(command) => {
                let output = execute_app_request(
                    &state,
                    StdioRequest::UserInput {
                        id: command.id,
                        request_id: command.request_id,
                        response: command.response,
                    },
                )
                .await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/cancel") => match read_json::<CancelRequest>(request).await {
            Ok(command) => {
                let output = execute_app_request(
                    &state,
                    StdioRequest::Cancel {
                        id: command.id,
                        target_id: command.target_id,
                    },
                )
                .await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/mode") => match read_json::<SetPermissionModeRequest>(request).await {
            Ok(command) => {
                let output = execute_app_request(
                    &state,
                    StdioRequest::SetPermissionMode {
                        id: command.id,
                        mode: command.mode,
                    },
                )
                .await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/model") => match read_json::<SetModelRequest>(request).await {
            Ok(command) => {
                let output = execute_app_request(
                    &state,
                    StdioRequest::SetModel {
                        id: command.id,
                        model: command.model,
                    },
                )
                .await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/effort") => match read_json::<SetReasoningEffortRequest>(request).await {
            Ok(command) => {
                let output = execute_app_request(
                    &state,
                    StdioRequest::SetReasoningEffort {
                        id: command.id,
                        effort: command.effort,
                    },
                )
                .await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/reasoning") => {
            match read_json::<SetReasoningEnabledRequest>(request).await {
                Ok(command) => {
                    let output = execute_app_request(
                        &state,
                        StdioRequest::SetReasoningEnabled {
                            id: command.id,
                            enabled: command.enabled,
                        },
                    )
                    .await;
                    json_response(StatusCode::OK, &output)
                }
                Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
            }
        }
        (Method::POST, "/resume") => match read_json::<ResumeSessionRequest>(request).await {
            Ok(command) => {
                let output = execute_resume(&state, command.id, command.session_dir).await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/clear") => {
            let output = execute_app_request(&state, StdioRequest::ClearHistory { id: None }).await;
            json_response(StatusCode::OK, &output)
        }
        (Method::POST, "/shutdown") => {
            let output = execute_app_request(&state, StdioRequest::Shutdown { id: None }).await;
            json_response(StatusCode::OK, &output)
        }
        _ => error_response(StatusCode::NOT_FOUND, "unknown app-server HTTP endpoint"),
    };

    Ok(response)
}

async fn read_json<T: DeserializeOwned>(request: Request<Incoming>) -> Result<T> {
    let bytes = request
        .into_body()
        .collect()
        .await
        .map_err(|error| anyhow!("could not read request body: {error}"))?
        .to_bytes();
    serde_json::from_slice(&bytes).map_err(anyhow::Error::from)
}

async fn execute_app_request(state: &HttpAppState, request: StdioRequest) -> StdioOutput {
    let id = request.id();
    let result = match request {
        StdioRequest::Send { id, text } => execute_send(state, id, text).await.and_then(|output| {
            serde_json::to_value(output)
                .map(Some)
                .map_err(anyhow::Error::from)
        }),
        StdioRequest::ClearHistory { .. } => state
            .current_server()
            .await
            .clear_history()
            .await
            .map(|_| None),
        StdioRequest::Approval {
            approval_id,
            approved,
            note,
            cache,
            ..
        } => state
            .current_server()
            .await
            .respond_approval(&approval_id, approved, note, cache)
            .await
            .map(|_| None),
        StdioRequest::UserInput {
            request_id,
            response,
            ..
        } => state
            .current_server()
            .await
            .respond_user_input(&request_id, response)
            .await
            .map(|_| None),
        StdioRequest::Cancel { target_id, .. } => {
            execute_cancel(state, &target_id).await.map(|_| None)
        }
        StdioRequest::SetPermissionMode { mode, .. } => {
            state.current_server().await.set_permission_mode(mode).await;
            Ok(Some(json!({ "mode": mode })))
        }
        StdioRequest::SetModel { model, .. } => {
            state
                .current_server()
                .await
                .set_model_name(model.clone())
                .await;
            Ok(Some(json!({ "model": model })))
        }
        StdioRequest::SetReasoningEffort { effort, .. } => {
            state
                .current_server()
                .await
                .set_reasoning_effort(effort.clone())
                .await;
            Ok(Some(json!({ "effort": effort })))
        }
        StdioRequest::SetReasoningEnabled { enabled, .. } => {
            state
                .current_server()
                .await
                .set_reasoning_enabled(enabled)
                .await;
            Ok(Some(json!({ "enabled": enabled })))
        }
        StdioRequest::ConfigSummary { .. } => {
            Ok(Some(state.current_server().await.config_summary().await))
        }
        StdioRequest::Shutdown { .. } => {
            state.current_server().await.shutdown().await;
            let _ = state.shutdown.send(());
            Ok(None)
        }
        _ => Err(anyhow!("unsupported StdioRequest variant")),
    };
    command_response(id, result)
}

async fn execute_send(
    state: &HttpAppState,
    id: Option<String>,
    text: String,
) -> Result<AgentOutput> {
    let cancellation = CancellationToken::new();
    if let Some(turn_id) = id.as_deref() {
        let mut running_turns = state.running_turns.lock().await;
        if running_turns.contains_key(turn_id) {
            return Err(anyhow!("turn id is already running: {turn_id}"));
        }
        running_turns.insert(turn_id.to_owned(), cancellation.clone());
    }

    let result = state
        .current_server()
        .await
        .send_user_message_with_cancellation(text, cancellation)
        .await;

    if let Some(turn_id) = id.as_deref() {
        state.running_turns.lock().await.remove(turn_id);
    }
    result
}

async fn execute_resume(
    state: &HttpAppState,
    id: Option<String>,
    session_dir: PathBuf,
) -> StdioOutput {
    let result = resume_session(state, session_dir).await.map(Some);
    command_response(id, result)
}

async fn resume_session(state: &HttpAppState, session_dir: PathBuf) -> Result<Value> {
    if !state.running_turns.lock().await.is_empty() {
        return Err(anyhow!(
            "cannot resume another session while a turn is running"
        ));
    }

    let current = state.current_server().await;
    current
        .cancel_pending_approvals("session switched by client".to_owned())
        .await;
    current
        .cancel_pending_user_inputs("session switched by client".to_owned())
        .await;

    let next = AgentAppServer::launch_resumed(
        (*current.config).clone(),
        current.cwd.clone(),
        current.config_path.as_deref(),
        session_dir,
    )?;
    let summary = next.config_summary().await;
    *state.server.lock().await = next;
    Ok(summary)
}

async fn execute_cancel(state: &HttpAppState, target_id: &str) -> Result<()> {
    let cancellation = state
        .running_turns
        .lock()
        .await
        .remove(target_id)
        .ok_or_else(|| anyhow!("unknown or completed turn id: {target_id}"))?;
    cancellation.cancel();
    state
        .current_server()
        .await
        .cancel_pending_approvals("turn canceled by client".to_owned())
        .await;
    state
        .current_server()
        .await
        .cancel_pending_user_inputs("turn canceled by client".to_owned())
        .await;
    Ok(())
}

fn command_response(id: Option<String>, result: Result<Option<Value>>) -> StdioOutput {
    match result {
        Ok(output) => StdioOutput::Response {
            id,
            ok: true,
            output,
            error: None,
        },
        Err(error) => StdioOutput::Response {
            id,
            ok: false,
            output: None,
            error: Some(format!("{error:#}")),
        },
    }
}

async fn sse_response(state: HttpAppState) -> HttpResponse {
    let server = state.current_server().await;
    let mut events = server.subscribe();
    let body = StreamBody::new(stream! {
        if let Err(error) = server.start_session().await {
            let output = command_response(None, Err(error));
            yield Ok::<Frame<Bytes>, Infallible>(Frame::data(encode_sse_output(&output)));
            return;
        }

        loop {
            match events.recv().await {
                Ok(event) => {
                    let should_stop = matches!(event, AppServerEvent::Shutdown);
                    let output = StdioOutput::Event {
                        event: Box::new(event),
                    };
                    yield Ok(Frame::data(encode_sse_output(&output)));
                    if should_stop {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    let output = command_response(
                        None,
                        Err(anyhow!("app-server event stream lagged by {count} events")),
                    );
                    yield Ok(Frame::data(encode_sse_output(&output)));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
    .boxed_unsync();

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header(CACHE_CONTROL, "no-cache")
        .header(CONNECTION, "keep-alive")
        .body(body)
        .expect("sse response is valid");
    add_cors_headers(&mut response);
    response
}

fn encode_sse_output(output: &StdioOutput) -> Bytes {
    let data = serde_json::to_string(output).unwrap_or_else(|error| {
        serde_json::to_string(&json!({
            "type": "response",
            "id": null,
            "ok": false,
            "output": null,
            "error": format!("{error:#}"),
        }))
        .expect("fallback response serializes")
    });
    Bytes::from(format!("event: output\ndata: {data}\n\n"))
}

fn json_response<T: serde::Serialize>(status: StatusCode, body: &T) -> HttpResponse {
    match serde_json::to_vec(body) {
        Ok(body) => response_with_body(status, "application/json", Bytes::from(body)),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error:#}")),
    }
}

fn error_response(status: StatusCode, message: &str) -> HttpResponse {
    response_with_body(
        status,
        "application/json",
        Bytes::from(
            serde_json::to_vec(&json!({
                "ok": false,
                "error": message,
            }))
            .expect("error response serializes"),
        ),
    )
}

fn empty_response(status: StatusCode) -> HttpResponse {
    response_with_body(status, "text/plain; charset=utf-8", Bytes::new())
}

fn response_with_body(status: StatusCode, content_type: &'static str, body: Bytes) -> HttpResponse {
    let body = Full::new(body).boxed_unsync();
    let mut response = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(body)
        .expect("HTTP response is valid");
    add_cors_headers(&mut response);
    response
}

fn add_cors_headers(response: &mut HttpResponse) {
    let headers = response.headers_mut();
    headers.insert(
        "access-control-allow-origin",
        "*".parse().expect("valid header"),
    );
    headers.insert(
        "access-control-allow-methods",
        "GET, POST, OPTIONS".parse().expect("valid header"),
    );
    headers.insert(
        "access-control-allow-headers",
        "content-type".parse().expect("valid header"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AppConfig;

    #[tokio::test]
    async fn request_dispatch_sets_permission_mode() {
        let cwd = tempfile::tempdir().expect("cwd");
        let server = AgentAppServer::launch(AppConfig::default(), cwd.path().to_path_buf(), None)
            .expect("app server");
        let (shutdown, _) = broadcast::channel(1);
        let state = HttpAppState::new(server.clone(), shutdown);

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
        let state = HttpAppState::new(server.clone(), shutdown);

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
        let state = HttpAppState::new(server.clone(), shutdown);

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
    async fn cancel_unknown_turn_returns_protocol_error() {
        let cwd = tempfile::tempdir().expect("cwd");
        let server = AgentAppServer::launch(AppConfig::default(), cwd.path().to_path_buf(), None)
            .expect("app server");
        let (shutdown, _) = broadcast::channel(1);
        let state = HttpAppState::new(server.clone(), shutdown);

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
}
