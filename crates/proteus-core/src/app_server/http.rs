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
    body::{Body, Frame},
    header::{AUTHORIZATION, CACHE_CONTROL, CONNECTION, CONTENT_TYPE, HeaderValue, ORIGIN},
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

const SESSION_TOKEN_HEADERS: [&str; 2] = ["x-proteus-session", "x-proteus-session-token"];
const SESSION_TOKEN_QUERY: &str = "token";
const SESSION_TOKEN_QUERY_ALIASES: [&str; 3] = ["session", "session_token", "proteus_session"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpServerConfig {
    pub bind: SocketAddr,
    pub session_token: String,
    pub allowed_origins: Vec<String>,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787),
            session_token: new_session_token(),
            allowed_origins: default_allowed_origins(),
        }
    }
}

#[derive(Clone)]
struct HttpAppState {
    server: Arc<Mutex<AppServerHandle>>,
    running_turns: Arc<Mutex<HashMap<String, CancellationToken>>>,
    shutdown: broadcast::Sender<()>,
    security: HttpSecurity,
}

impl HttpAppState {
    fn new(
        server: AppServerHandle,
        shutdown: broadcast::Sender<()>,
        security: HttpSecurity,
    ) -> Self {
        Self {
            server: Arc::new(Mutex::new(server)),
            running_turns: Arc::new(Mutex::new(HashMap::new())),
            shutdown,
            security,
        }
    }

    async fn current_server(&self) -> AppServerHandle {
        self.server.lock().await.clone()
    }
}

#[derive(Clone)]
struct HttpSecurity {
    session_token: Arc<str>,
    allowed_origins: Arc<[String]>,
}

impl HttpSecurity {
    fn from_config(config: &HttpServerConfig) -> Self {
        Self {
            session_token: Arc::from(config.session_token.as_str()),
            allowed_origins: Arc::from(config.allowed_origins.clone().into_boxed_slice()),
        }
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
    let security = HttpSecurity::from_config(&http_config);
    let state = HttpAppState::new(server, shutdown, security);
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

async fn route_request<B>(
    state: HttpAppState,
    request: Request<B>,
) -> Result<HttpResponse, Infallible>
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: std::fmt::Display,
{
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let cors_origin = match validate_origin(&request, &state.security) {
        Ok(origin) => origin,
        Err(response) => return Ok(*response),
    };

    if method == Method::OPTIONS {
        return Ok(options_response(&request, cors_origin));
    }

    if endpoint_requires_auth(&method, &path) && !request_has_valid_token(&request, &state.security)
    {
        let mut response = error_response(
            StatusCode::UNAUTHORIZED,
            "missing or invalid app-server session token",
        );
        add_cors_headers(&mut response, cors_origin.as_ref());
        return Ok(response);
    }

    let response = match (method, path.as_str()) {
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
        (Method::POST, "/request") => match read_json::<StdioRequest, _>(request).await {
            Ok(command) => {
                json_response(StatusCode::OK, &execute_app_request(&state, command).await)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/send") => match read_json::<SendRequest, _>(request).await {
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
        (Method::POST, "/approval") => match read_json::<ApprovalRequest, _>(request).await {
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
        (Method::POST, "/user-input") => match read_json::<UserInputRequest, _>(request).await {
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
        (Method::POST, "/cancel") => match read_json::<CancelRequest, _>(request).await {
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
        (Method::POST, "/mode") => match read_json::<SetPermissionModeRequest, _>(request).await {
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
        (Method::POST, "/model") => match read_json::<SetModelRequest, _>(request).await {
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
        (Method::POST, "/effort") => match read_json::<SetReasoningEffortRequest, _>(request).await
        {
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
            match read_json::<SetReasoningEnabledRequest, _>(request).await {
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
        (Method::POST, "/resume") => match read_json::<ResumeSessionRequest, _>(request).await {
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
        (Method::POST, "/reload-tools") => {
            let output = execute_app_request(&state, StdioRequest::ReloadTools { id: None }).await;
            json_response(StatusCode::OK, &output)
        }
        (Method::POST, "/shutdown") => {
            let output = execute_app_request(&state, StdioRequest::Shutdown { id: None }).await;
            json_response(StatusCode::OK, &output)
        }
        _ => error_response(StatusCode::NOT_FOUND, "unknown app-server HTTP endpoint"),
    };

    let mut response = response;
    add_cors_headers(&mut response, cors_origin.as_ref());
    Ok(response)
}

fn endpoint_requires_auth(method: &Method, path: &str) -> bool {
    !matches!(
        (method, path),
        (&Method::OPTIONS, _) | (&Method::GET, "/health")
    )
}

fn validate_origin<B>(
    request: &Request<B>,
    security: &HttpSecurity,
) -> Result<Option<HeaderValue>, Box<HttpResponse>> {
    let Some(origin) = request.headers().get(ORIGIN) else {
        return Ok(None);
    };
    let Ok(origin_text) = origin.to_str() else {
        return Err(Box::new(error_response(
            StatusCode::FORBIDDEN,
            "request origin is not allowed",
        )));
    };
    if is_allowed_origin(origin_text, &security.allowed_origins) {
        return Ok(Some(origin.clone()));
    }
    Err(Box::new(error_response(
        StatusCode::FORBIDDEN,
        "request origin is not allowed",
    )))
}

fn is_allowed_origin(origin: &str, allowed_origins: &[String]) -> bool {
    allowed_origins
        .iter()
        .any(|allowed| origin.eq_ignore_ascii_case(allowed))
}

fn request_has_valid_token<B>(request: &Request<B>, security: &HttpSecurity) -> bool {
    SESSION_TOKEN_HEADERS.iter().any(|header| {
        request
            .headers()
            .get(*header)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|token| token_matches(token, &security.session_token))
    }) || request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token)
        .is_some_and(|token| token_matches(token, &security.session_token))
        || request
            .uri()
            .query()
            .is_some_and(|query| query_has_valid_token(query, &security.session_token))
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() {
        Some(token)
    } else {
        None
    }
}

fn query_has_valid_token(query: &str, expected: &str) -> bool {
    query.split('&').any(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode_query_value(value);
        (key == SESSION_TOKEN_QUERY || SESSION_TOKEN_QUERY_ALIASES.contains(&key))
            && token_matches(value.as_ref(), expected)
    })
}

fn percent_decode_query_value(value: &str) -> std::borrow::Cow<'_, str> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut changed = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                if let Some(byte) = hex_pair(bytes[index + 1], bytes[index + 2]) {
                    decoded.push(byte);
                    changed = true;
                    index += 3;
                } else {
                    decoded.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    if changed {
        String::from_utf8(decoded)
            .map(std::borrow::Cow::Owned)
            .unwrap_or(std::borrow::Cow::Borrowed(value))
    } else {
        std::borrow::Cow::Borrowed(value)
    }
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    Some(hex_digit(high)? << 4 | hex_digit(low)?)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn token_matches(provided: &str, expected: &str) -> bool {
    let provided = provided.as_bytes();
    let expected = expected.as_bytes();
    if provided.len() != expected.len() {
        return false;
    }
    provided
        .iter()
        .zip(expected.iter())
        .fold(0_u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

fn options_response<B>(request: &Request<B>, cors_origin: Option<HeaderValue>) -> HttpResponse {
    let mut response = if request
        .headers()
        .get("access-control-request-method")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|method| !matches!(method, "GET" | "POST" | "OPTIONS"))
    {
        error_response(StatusCode::METHOD_NOT_ALLOWED, "HTTP method is not allowed")
    } else {
        empty_response(StatusCode::NO_CONTENT)
    };
    add_cors_headers(&mut response, cors_origin.as_ref());
    response
}

fn new_session_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn default_allowed_origins() -> Vec<String> {
    vec![
        "http://127.0.0.1:1420".to_owned(),
        "http://localhost:1420".to_owned(),
    ]
}

async fn read_json<T, B>(request: Request<B>) -> Result<T>
where
    T: DeserializeOwned,
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: std::fmt::Display,
{
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
        StdioRequest::ReloadTools { .. } => state
            .current_server()
            .await
            .reload_tools()
            .await
            .and_then(|report| {
                serde_json::to_value(report)
                    .map(Some)
                    .map_err(anyhow::Error::from)
            }),
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

    let config = current.config.read().await.clone();
    let next = AgentAppServer::launch_resumed(
        config,
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
        yield Ok::<Frame<Bytes>, Infallible>(Frame::data(Bytes::from_static(b": connected\n\n")));

        if let Err(error) = server.start_session().await {
            let output = command_response(None, Err(error));
            yield Ok(Frame::data(encode_sse_output(&output)));
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

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header(CACHE_CONTROL, "no-cache")
        .header(CONNECTION, "keep-alive")
        .body(body)
        .expect("sse response is valid")
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
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(body)
        .expect("HTTP response is valid")
}

fn add_cors_headers(response: &mut HttpResponse, origin: Option<&HeaderValue>) {
    let Some(origin) = origin else {
        return;
    };
    let headers = response.headers_mut();
    headers.insert("access-control-allow-origin", origin.clone());
    headers.insert(
        "access-control-allow-methods",
        "GET, POST, OPTIONS".parse().expect("valid header"),
    );
    headers.insert(
        "access-control-allow-headers",
        "authorization, content-type, x-proteus-session, x-proteus-session-token"
            .parse()
            .expect("valid header"),
    );
    headers.insert(
        "access-control-allow-credentials",
        "true".parse().expect("valid header"),
    );
    headers.insert("vary", "origin".parse().expect("valid header"));
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use super::*;
    use crate::contracts::UserInputAnswer;
    use crate::core::AppConfig;

    fn empty_body() -> Full<Bytes> {
        Full::new(Bytes::new())
    }

    fn test_security() -> HttpSecurity {
        let mut allowed_origins = default_allowed_origins();
        allowed_origins.push("https://app.example.test".to_owned());
        HttpSecurity {
            session_token: Arc::from("session-secret"),
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

    #[test]
    fn protected_endpoints_require_session_token_except_health_and_preflight() {
        let protected = [
            (Method::GET, "/events"),
            (Method::GET, "/config"),
            (Method::GET, "/sessions"),
            (Method::GET, "/history"),
            (Method::POST, "/request"),
            (Method::POST, "/send"),
            (Method::POST, "/approval"),
            (Method::POST, "/user-input"),
            (Method::POST, "/cancel"),
            (Method::POST, "/mode"),
            (Method::POST, "/model"),
            (Method::POST, "/effort"),
            (Method::POST, "/reasoning"),
            (Method::POST, "/resume"),
            (Method::POST, "/clear"),
            (Method::POST, "/reload-tools"),
            (Method::POST, "/shutdown"),
        ];

        for (method, path) in protected {
            assert!(
                endpoint_requires_auth(&method, path),
                "{method} {path} must require auth"
            );
        }
        assert!(!endpoint_requires_auth(&Method::GET, "/health"));
        assert!(!endpoint_requires_auth(&Method::OPTIONS, "/config"));
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
        server
            .pending_approvals
            .lock()
            .await
            .insert(approval_id.clone(), approval_tx);
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
        server
            .pending_user_inputs
            .lock()
            .await
            .insert(request_id.clone(), input_tx);
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
        state
            .running_turns
            .lock()
            .await
            .insert(turn_id.clone(), cancellation.clone());

        let (approval_tx, approval_rx) = tokio::sync::oneshot::channel();
        let approval_id = "approval-cancel".to_owned();
        server
            .pending_approvals
            .lock()
            .await
            .insert(approval_id, approval_tx);

        let (input_tx, input_rx) = tokio::sync::oneshot::channel();
        let request_id = "input-cancel".to_owned();
        server
            .pending_user_inputs
            .lock()
            .await
            .insert(request_id, input_tx);

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
}
