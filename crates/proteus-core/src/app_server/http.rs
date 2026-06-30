use std::{
    collections::HashSet,
    convert::Infallible,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use async_stream::stream;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited, StreamBody, combinators::UnsyncBoxBody};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Body, Frame},
    header::{CACHE_CONTROL, CONNECTION, CONTENT_TYPE, HeaderValue},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use proteus_contracts::app_protocol::AppSessionActivityStatus;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{broadcast, oneshot},
};

use crate::{
    contracts::CancellationToken,
    core::{
        AppConfig, SessionStore, canonicalize_session_dir_path, render_topology_map,
        render_topology_mermaid, render_topology_runtime_mermaid, render_topology_runtime_path,
    },
    domain::{AgentOutput, PermissionMode},
};

use super::{
    AgentAppServer, AppServerEvent, AppServerHandle, AppSessionActivity, AppTranscriptMessage,
    protocol::{StdioOutput, StdioRequest},
    transcript_messages,
};

mod config;
mod requests;
mod security;
mod state;

pub use config::HttpServerConfig;
use config::new_request_id;
use requests::{
    ApprovalRequest, CancelRequest, DeleteSessionRequest, NewSessionRequest, ResumeSessionRequest,
    SendRequest, SetConfigBuilderRequest, SetModelRequest, SetPermissionModeRequest,
    SetReasoningEffortRequest, SetReasoningEnabledRequest, SetWebConfigRequest, UserInputRequest,
};
use security::{
    HttpSecurity, request_has_valid_token, request_requires_session_token, validate_origin,
};
use state::{HttpAppState, RunningTurn, session_key as canonical_session_key};

type HttpBody = UnsyncBoxBody<Bytes, Infallible>;
type HttpResponse = Response<HttpBody>;

const SSE_HEARTBEAT_SECS: u64 = 15;
const MAX_JSON_BODY_BYTES: usize = 2 * 1024 * 1024;

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
        AgentAppServer::launch_or_resume_latest(config, cwd, config_path.as_deref())?
    };
    let (shutdown, mut shutdown_rx) = broadcast::channel(1);
    let security = HttpSecurity::from_config(&http_config);
    let state = HttpAppState::new(server, shutdown, security);
    let listener = TcpListener::bind(http_config.bind).await?;
    println!(
        "Proteus app-server HTTP listening on http://{}",
        listener.local_addr()?
    );
    if http_config.require_session_token {
        println!("HTTP session token auth: enabled");
    } else {
        println!("HTTP session token auth: disabled for local dogfood");
    }
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let state = state.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let service = service_fn(move |request| route_request(state.clone(), request));
                    if let Err(error) = http1::Builder::new().serve_connection(io, service).await {
                        if should_log_http_connection_error(&error) {
                            eprintln!("app-server HTTP connection error: {error}");
                        }
                    }
                });
            }
            _ = shutdown_rx.recv() => break,
        }
    }
    Ok(())
}

fn should_log_http_connection_error(error: &hyper::Error) -> bool {
    !(error.is_closed() || error.is_incomplete_message())
}

async fn route_request<B>(
    state: HttpAppState,
    request: Request<B>,
) -> Result<HttpResponse, Infallible>
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().map(str::to_owned);
    let cors_origin = match validate_origin(&request, &state.security) {
        Ok(origin) => origin,
        Err(response) => return Ok(*response),
    };

    if method == Method::OPTIONS {
        return Ok(options_response(&request, cors_origin));
    }

    if request_requires_session_token(&method, &path, &state.security)
        && !request_has_valid_token(&request, &state.security)
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
        (Method::GET, "/config/builder") => {
            let snapshot = state.current_server().await.config_builder_snapshot().await;
            json_response(StatusCode::OK, &snapshot)
        }
        (Method::GET, "/inspect/topology") => {
            let snapshot = state.current_server().await.topology_snapshot().await;
            json_response(StatusCode::OK, &snapshot)
        }
        (Method::GET, "/inspect/topology.mmd") => {
            let snapshot = state.current_server().await.topology_snapshot().await;
            text_response(StatusCode::OK, render_topology_mermaid(&snapshot))
        }
        (Method::GET, "/inspect/topology.map") => {
            let snapshot = state.current_server().await.topology_snapshot().await;
            text_response(StatusCode::OK, render_topology_map(&snapshot))
        }
        (Method::GET, "/inspect/topology.runtime") => {
            let snapshot = state.current_server().await.topology_snapshot().await;
            text_response(StatusCode::OK, render_topology_runtime_path(&snapshot))
        }
        (Method::GET, "/inspect/topology.runtime.mmd") => {
            let snapshot = state.current_server().await.topology_snapshot().await;
            text_response(StatusCode::OK, render_topology_runtime_mermaid(&snapshot))
        }
        (Method::GET, "/sessions") => match session_summaries_json(&state, false).await {
            Ok(sessions) => json_response(StatusCode::OK, &sessions),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error:#}")),
        },
        (Method::GET, "/sessions/current") => match session_summaries_json(&state, true).await {
            Ok(sessions) => json_response(StatusCode::OK, &sessions),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error:#}")),
        },
        (Method::GET, "/pending") => {
            let pending = state.current_server().await.pending_requests().await;
            json_response(StatusCode::OK, &pending)
        }
        (Method::GET, "/history") => match history_json(&state, query.as_deref()).await {
            Ok(transcript) => json_response(StatusCode::OK, &transcript),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error:#}")),
        },
        (Method::GET, "/context") => match context_map_json(&state, query.as_deref()).await {
            Ok(snapshot) => json_response(StatusCode::OK, &snapshot),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error:#}")),
        },
        (Method::POST, "/request") => match read_json::<StdioRequest, _>(request).await {
            Ok(command) => {
                json_response(StatusCode::OK, &execute_app_request(&state, command).await)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/send") => match read_json::<SendRequest, _>(request).await {
            Ok(command) => {
                let id = command.id;
                let output = command_response(
                    id.clone(),
                    execute_send(&state, id, command.text, command.session_dir)
                        .await
                        .and_then(|output| {
                            serde_json::to_value(output)
                                .map(Some)
                                .map_err(anyhow::Error::from)
                        }),
                );
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/send-async") => match read_json::<SendRequest, _>(request).await {
            Ok(command) => {
                let output =
                    execute_send_async(&state, command.id, command.text, command.session_dir).await;
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
                let output = execute_set_permission_mode(
                    &state,
                    command.id,
                    command.mode,
                    command.session_dir,
                )
                .await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/model") => match read_json::<SetModelRequest, _>(request).await {
            Ok(command) => {
                let output =
                    execute_set_model(&state, command.id, command.model, command.session_dir).await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/effort") => match read_json::<SetReasoningEffortRequest, _>(request).await
        {
            Ok(command) => {
                let output = execute_set_reasoning_effort(
                    &state,
                    command.id,
                    command.effort,
                    command.session_dir,
                )
                .await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/reasoning") => {
            match read_json::<SetReasoningEnabledRequest, _>(request).await {
                Ok(command) => {
                    let output = execute_set_reasoning_enabled(
                        &state,
                        command.id,
                        command.enabled,
                        command.session_dir,
                    )
                    .await;
                    json_response(StatusCode::OK, &output)
                }
                Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
            }
        }
        (Method::POST, "/config/builder") => {
            match read_json::<SetConfigBuilderRequest, _>(request).await {
                Ok(command) => match state
                    .current_server()
                    .await
                    .set_config_builder(command.modules, command.module_config)
                    .await
                {
                    Ok(snapshot) => json_response(StatusCode::OK, &snapshot),
                    Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
                },
                Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
            }
        }
        (Method::POST, "/config/web") => match read_json::<SetWebConfigRequest, _>(request).await {
            Ok(command) => {
                let output =
                    execute_set_web_config(&state, command.id, command.tool_cards_collapsed).await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/resume") => match read_json::<ResumeSessionRequest, _>(request).await {
            Ok(command) => {
                let output = execute_resume(&state, command.id, command.session_dir).await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/new-session") => match read_json::<NewSessionRequest, _>(request).await {
            Ok(command) => {
                let output = execute_new_session(&state, command.id).await;
                json_response(StatusCode::OK, &output)
            }
            Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
        },
        (Method::POST, "/delete-session") => {
            match read_json::<DeleteSessionRequest, _>(request).await {
                Ok(command) => {
                    let output =
                        execute_delete_session(&state, command.id, command.session_dir).await;
                    json_response(StatusCode::OK, &output)
                }
                Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
            }
        }
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

async fn session_summaries_json(
    state: &HttpAppState,
    current_workspace_only: bool,
) -> Result<Vec<Value>> {
    let current = state.current_server().await;
    let summaries = if current_workspace_only {
        current.workspace_session_summaries()?
    } else {
        current.session_summaries()?
    };
    let activity_by_dir = state.activity_by_session_dir().await;
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    let current_session_key = current.session_dir_path().map(canonical_session_key);

    for summary in summaries {
        let session_dir = summary.session_dir.clone();
        let session_key = canonical_session_key(session_dir.clone());
        seen.insert(session_key.clone());
        let mut value = serde_json::to_value(&summary)?;
        if let Some(activity) = activity_by_dir.get(&session_key)
            && let Value::Object(fields) = &mut value
        {
            fields.insert("activity".to_owned(), serde_json::to_value(activity)?);
        }
        values.push(value);
    }

    for server in state.all_servers().await {
        let Some(session_dir) = server.session_dir_path() else {
            continue;
        };
        let session_key = canonical_session_key(session_dir.clone());
        if seen.contains(&session_key) {
            continue;
        }
        if current_workspace_only && !super::paths_equal(server.cwd_path(), current.cwd_path()) {
            continue;
        }
        let activity = state.activity_for_server(&server).await;
        let include_empty_idle = Some(&session_key) == current_session_key.as_ref();
        if let Some(value) =
            known_session_summary_value(&server, &session_dir, activity, include_empty_idle).await?
        {
            seen.insert(session_key);
            values.push(value);
        }
    }

    values.sort_by(|left, right| {
        summary_updated_at_ms(right)
            .cmp(&summary_updated_at_ms(left))
            .then_with(|| summary_session_dir(right).cmp(&summary_session_dir(left)))
    });
    Ok(values)
}

async fn known_session_summary_value(
    server: &AppServerHandle,
    session_dir: &Path,
    activity: AppSessionActivity,
    include_empty_idle: bool,
) -> Result<Option<Value>> {
    let transcript = server.transcript().await;
    let message_count = transcript.len();
    if message_count == 0 && session_activity_is_idle(&activity) && !include_empty_idle {
        return Ok(None);
    }

    Ok(Some(json!({
        "session_dir": session_dir.to_path_buf(),
        "session_id": null,
        "workspace_path": server.cwd_path().to_path_buf(),
        "message_count": message_count,
        "updated_at_ms": current_time_ms(),
        "preview": transcript_preview(&transcript),
        "resumable": true,
        "activity": activity,
    })))
}

fn session_activity_is_idle(activity: &AppSessionActivity) -> bool {
    activity.status == AppSessionActivityStatus::Idle
        && activity.running_turns == 0
        && activity.running_turn_ids.is_empty()
        && activity.pending_approvals == 0
        && activity.pending_user_inputs == 0
}

fn transcript_preview(transcript: &[crate::app_server::AppTranscriptMessage]) -> Option<String> {
    transcript
        .iter()
        .find(|message| message.role == "user" && !message.text.trim().is_empty())
        .or_else(|| {
            transcript
                .iter()
                .find(|message| !message.text.trim().is_empty())
        })
        .map(|message| truncate_session_preview(message.text.trim()))
}

fn truncate_session_preview(text: &str) -> String {
    let limit = 160;
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        format!("{}...", text.chars().take(limit).collect::<String>())
    }
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn summary_updated_at_ms(value: &Value) -> u64 {
    value
        .get("updated_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn summary_session_dir(value: &Value) -> String {
    value
        .get("session_dir")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

async fn history_json(
    state: &HttpAppState,
    query: Option<&str>,
) -> Result<Vec<AppTranscriptMessage>> {
    let Some(session_dir) = query_path_param(query, "session_dir")? else {
        return Ok(state.current_server().await.transcript().await);
    };
    let session_dir = canonicalize_session_dir_path(session_dir)?;
    if let Some(server) = state.server_for_session_dir(&session_dir).await {
        return Ok(server.transcript().await);
    }

    let messages = SessionStore::from_session_dir(session_dir).load_messages()?;
    Ok(transcript_messages(&messages))
}

async fn context_map_json(
    state: &HttpAppState,
    query: Option<&str>,
) -> Result<super::AppContextMapSnapshot> {
    let Some(session_dir) = query_path_param(query, "session_dir")? else {
        let server = state.current_server().await;
        let activity = state.activity_for_server(&server).await;
        return server.context_map_snapshot(Some(activity)).await;
    };
    let session_dir = canonicalize_session_dir_path(session_dir)?;
    if let Some(server) = state.server_for_session_dir(&session_dir).await {
        let activity = state.activity_for_server(&server).await;
        return server.context_map_snapshot(Some(activity)).await;
    }

    state
        .current_server()
        .await
        .context_map_snapshot_for_session_dir(session_dir, None)
        .await
}

async fn server_for_optional_session(
    state: &HttpAppState,
    session_dir: Option<PathBuf>,
) -> Result<AppServerHandle> {
    let Some(session_dir) = session_dir else {
        return Ok(state.current_server().await);
    };
    let session_dir = canonicalize_session_dir_path(session_dir)?;
    state
        .server_for_session_dir(&session_dir)
        .await
        .ok_or_else(|| {
            anyhow!(
                "session is not active; resume it first: {}",
                session_dir.display()
            )
        })
}

fn query_path_param(query: Option<&str>, name: &str) -> Result<Option<PathBuf>> {
    let Some(query) = query else {
        return Ok(None);
    };
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == name {
            return Ok(Some(PathBuf::from(percent_decode_query_value(value)?)));
        }
    }
    Ok(None)
}

fn percent_decode_query_value(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                if let (Some(high), Some(low)) =
                    (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
                {
                    decoded.push((high << 4) | low);
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
    String::from_utf8(decoded).map_err(anyhow::Error::from)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

async fn read_json<T, B>(request: Request<B>) -> Result<T>
where
    T: DeserializeOwned,
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let bytes = Limited::new(request.into_body(), MAX_JSON_BODY_BYTES)
        .collect()
        .await
        .map_err(|error| {
            anyhow!("could not read request body within {MAX_JSON_BODY_BYTES} bytes: {error}")
        })?
        .to_bytes();
    serde_json::from_slice(&bytes).map_err(anyhow::Error::from)
}

async fn execute_app_request(state: &HttpAppState, request: StdioRequest) -> StdioOutput {
    let id = request.id();
    let result = match request {
        StdioRequest::Send { id, text } => {
            execute_send(state, id, text, None)
                .await
                .and_then(|output| {
                    serde_json::to_value(output)
                        .map(Some)
                        .map_err(anyhow::Error::from)
                })
        }
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
        } => {
            let server = match state.server_for_pending_approval(&approval_id).await {
                Some(server) => server,
                None => state.current_server().await,
            };
            let result = server
                .respond_approval(&approval_id, approved, note, cache)
                .await
                .map(|_| None);
            state.emit_session_activity_for_server(&server).await;
            result
        }
        StdioRequest::UserInput {
            request_id,
            response,
            ..
        } => {
            let server = match state.server_for_pending_user_input(&request_id).await {
                Some(server) => server,
                None => state.current_server().await,
            };
            let result = server
                .respond_user_input(&request_id, response)
                .await
                .map(|_| None);
            state.emit_session_activity_for_server(&server).await;
            result
        }
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
            shutdown_all_servers(state).await;
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
    session_dir: Option<PathBuf>,
) -> Result<AgentOutput> {
    let cancellation = CancellationToken::new();
    let server = server_for_optional_session(state, session_dir).await?;
    let receiver = spawn_send_turn(state, server, id, text, cancellation).await?;
    receiver
        .await
        .map_err(|_| anyhow!("send turn task dropped before completion"))?
}

async fn spawn_send_turn(
    state: &HttpAppState,
    server: AppServerHandle,
    turn_id: Option<String>,
    text: String,
    cancellation: CancellationToken,
) -> Result<oneshot::Receiver<Result<AgentOutput>>> {
    let session_dir = server.session_dir_path();
    if let Some(turn_id) = turn_id.as_deref() {
        let session_key = session_dir.clone().map(canonical_session_key);
        let mut running_turns = state.running_turns.lock().await;
        if running_turns.contains_key(turn_id) {
            return Err(anyhow!("turn id is already running: {turn_id}"));
        }
        if let Some((existing_turn_id, _)) = running_turns
            .iter()
            .find(|(_, turn)| turn.session_dir == session_key)
        {
            return Err(anyhow!(
                "session already has a running turn: {existing_turn_id}"
            ));
        }
        running_turns.insert(
            turn_id.to_owned(),
            RunningTurn::new(cancellation.clone(), session_dir.clone()),
        );
    }
    state.emit_session_activity_for_server(&server).await;

    let (result_tx, result_rx) = oneshot::channel();
    let state_for_activity = state.clone();
    tokio::spawn(async move {
        let result = server
            .send_user_message_with_cancellation(text, cancellation)
            .await;
        if let Some(turn_id) = turn_id.as_deref() {
            state_for_activity
                .running_turns
                .lock()
                .await
                .remove(turn_id);
        }
        state_for_activity
            .emit_session_activity_for_server(&server)
            .await;
        let _ = result_tx.send(result);
    });
    Ok(result_rx)
}

async fn execute_send_async(
    state: &HttpAppState,
    id: Option<String>,
    text: String,
    session_dir: Option<PathBuf>,
) -> StdioOutput {
    let turn_id = id.unwrap_or_else(new_request_id);
    let cancellation = CancellationToken::new();
    let server = match server_for_optional_session(state, session_dir).await {
        Ok(server) => server,
        Err(error) => return command_response(Some(turn_id), Err(error)),
    };
    if let Err(error) =
        spawn_send_turn(state, server, Some(turn_id.clone()), text, cancellation).await
    {
        return command_response(Some(turn_id), Err(error));
    }

    command_response(
        Some(turn_id.clone()),
        Ok(Some(json!({
            "turn_id": turn_id,
            "accepted": true,
        }))),
    )
}

async fn execute_set_permission_mode(
    state: &HttpAppState,
    id: Option<String>,
    mode: PermissionMode,
    session_dir: Option<PathBuf>,
) -> StdioOutput {
    let result = async {
        let server = server_for_optional_session(state, session_dir).await?;
        server.set_permission_mode(mode).await;
        Ok(Some(json!({ "mode": mode })))
    }
    .await;
    command_response(id, result)
}

async fn execute_set_model(
    state: &HttpAppState,
    id: Option<String>,
    model: String,
    session_dir: Option<PathBuf>,
) -> StdioOutput {
    let result = async {
        let server = server_for_optional_session(state, session_dir).await?;
        server.set_model_name(model.clone()).await;
        Ok(Some(json!({ "model": model })))
    }
    .await;
    command_response(id, result)
}

async fn execute_set_web_config(
    state: &HttpAppState,
    id: Option<String>,
    tool_cards_collapsed: Option<bool>,
) -> StdioOutput {
    let result = async {
        let server = server_for_optional_session(state, None).await?;
        server.set_web_config(tool_cards_collapsed).await?;
        Ok(Some(json!({
            "web": { "tool_cards_collapsed": tool_cards_collapsed },
        })))
    }
    .await;
    command_response(id, result)
}

async fn execute_set_reasoning_effort(
    state: &HttpAppState,
    id: Option<String>,
    effort: Option<String>,
    session_dir: Option<PathBuf>,
) -> StdioOutput {
    let result = async {
        let server = server_for_optional_session(state, session_dir).await?;
        server.set_reasoning_effort(effort.clone()).await;
        Ok(Some(json!({ "effort": effort })))
    }
    .await;
    command_response(id, result)
}

async fn execute_set_reasoning_enabled(
    state: &HttpAppState,
    id: Option<String>,
    enabled: bool,
    session_dir: Option<PathBuf>,
) -> StdioOutput {
    let result = async {
        let server = server_for_optional_session(state, session_dir).await?;
        server.set_reasoning_enabled(enabled).await;
        Ok(Some(json!({ "enabled": enabled })))
    }
    .await;
    command_response(id, result)
}

async fn execute_resume(
    state: &HttpAppState,
    id: Option<String>,
    session_dir: PathBuf,
) -> StdioOutput {
    let result = resume_session(state, session_dir).await.map(Some);
    command_response(id, result)
}

async fn execute_new_session(state: &HttpAppState, id: Option<String>) -> StdioOutput {
    let result = new_session(state).await.map(Some);
    command_response(id, result)
}

async fn execute_delete_session(
    state: &HttpAppState,
    id: Option<String>,
    session_dir: PathBuf,
) -> StdioOutput {
    let result = delete_session(state, session_dir).await.map(Some);
    command_response(id, result)
}

async fn resume_session(state: &HttpAppState, session_dir: PathBuf) -> Result<Value> {
    let current = state.current_server().await;
    let session_dir = canonicalize_session_dir_path(session_dir)?;
    if let Some(existing) = state.server_for_session_dir(&session_dir).await {
        state.set_current_server(existing.clone()).await;
        return config_summary_with_activity(state, &existing).await;
    }

    let config = current.config.read().await.clone();
    let next = AgentAppServer::launch_resumed(
        config,
        current.cwd.clone(),
        current.config_path.as_deref(),
        session_dir,
    )?;
    state.set_current_server(next).await;
    let next = state.current_server().await;
    let summary = config_summary_with_activity(state, &next).await?;
    Ok(summary)
}

async fn config_summary_with_activity(
    state: &HttpAppState,
    server: &AppServerHandle,
) -> Result<Value> {
    let mut summary = server.config_summary().await;
    if let Value::Object(fields) = &mut summary {
        fields.insert(
            "activity".to_owned(),
            serde_json::to_value(state.activity_for_server(server).await)?,
        );
    }
    Ok(summary)
}

async fn delete_session(state: &HttpAppState, session_dir: PathBuf) -> Result<Value> {
    let current = state.current_server().await;
    let session_dir = canonicalize_session_dir_path(session_dir)?;
    let deleting_active = current.is_session_dir(&session_dir);
    let mut replacement_summary = None;
    if let Some(deleting_server) = state.server_for_session_dir(&session_dir).await {
        cancel_work_for_server(state, &deleting_server, "session deleted by client").await;
        state.remove_session_server(&session_dir).await;
    }

    if deleting_active {
        let current = state.current_server().await;
        let config = current.config.read().await.clone();
        let next =
            AgentAppServer::launch(config, current.cwd.clone(), current.config_path.as_deref())?;
        next.start_session().await?;
        replacement_summary = Some(next.config_summary().await);
        state.set_current_server(next).await;
    }

    let deleted = current.delete_workspace_session(session_dir).await?;
    Ok(json!({
        "deleted": deleted,
        "active_replaced": deleting_active,
        "session": replacement_summary,
    }))
}

async fn new_session(state: &HttpAppState) -> Result<Value> {
    let current = state.current_server().await;
    let config = current.config.read().await.clone();
    let next = AgentAppServer::launch(config, current.cwd.clone(), current.config_path.as_deref())?;
    next.start_session().await?;
    let summary = next.config_summary().await;
    state.set_current_server(next).await;
    Ok(summary)
}

async fn cancel_work_for_server(state: &HttpAppState, server: &AppServerHandle, note: &str) {
    let session_dir = server.session_dir_path().map(canonical_session_key);
    let active_turns = {
        let mut running_turns = state.running_turns.lock().await;
        let turn_ids = running_turns
            .iter()
            .filter_map(|(turn_id, turn)| {
                if turn.session_dir == session_dir {
                    Some(turn_id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        turn_ids
            .into_iter()
            .filter_map(|turn_id| running_turns.remove(&turn_id))
            .map(|turn| turn.cancellation)
            .collect::<Vec<_>>()
    };
    for cancellation in active_turns {
        cancellation.cancel();
    }

    server.cancel_pending_approvals(note.to_owned()).await;
    server.cancel_pending_user_inputs(note.to_owned()).await;
    state.emit_session_activity_for_server(server).await;
}

async fn shutdown_all_servers(state: &HttpAppState) {
    let cancellations = {
        let mut running_turns = state.running_turns.lock().await;
        running_turns
            .drain()
            .map(|(_, turn)| turn.cancellation)
            .collect::<Vec<_>>()
    };
    for cancellation in cancellations {
        cancellation.cancel();
    }

    for server in state.all_servers().await {
        server.shutdown().await;
        state.emit_session_activity_for_server(&server).await;
    }
}

async fn execute_cancel(state: &HttpAppState, target_id: &str) -> Result<()> {
    let turn = state
        .running_turns
        .lock()
        .await
        .remove(target_id)
        .ok_or_else(|| anyhow!("unknown or completed turn id: {target_id}"))?;
    turn.cancellation.cancel();
    let server = match turn.session_dir.as_deref() {
        Some(session_dir) => match state.server_for_session_dir(session_dir).await {
            Some(server) => server,
            None => state.current_server().await,
        },
        None => state.current_server().await,
    };
    server
        .cancel_pending_approvals("turn canceled by client".to_owned())
        .await;
    server
        .cancel_pending_user_inputs("turn canceled by client".to_owned())
        .await;
    state.emit_session_activity_for_server(&server).await;
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
    let mut activity_events = state.subscribe_activity();
    let body = StreamBody::new(stream! {
        yield Ok::<Frame<Bytes>, Infallible>(Frame::data(Bytes::from_static(b": connected\n\n")));

        if let Err(error) = server.start_session().await {
            let output = command_response(None, Err(error));
            yield Ok(Frame::data(encode_sse_output(&output)));
            return;
        }
        state.remember_server(server.clone()).await;
        state.emit_session_activity_for_server(&server).await;

        let mut heartbeat = tokio::time::interval(Duration::from_secs(SSE_HEARTBEAT_SECS));
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    yield Ok(Frame::data(Bytes::from_static(b": keep-alive\n\n")));
                }
                event = events.recv() => {
                    match event {
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
                event = activity_events.recv() => {
                    match event {
                        Ok(event) => {
                            let output = StdioOutput::Event {
                                event: Box::new(event),
                            };
                            yield Ok(Frame::data(encode_sse_output(&output)));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                            let output = command_response(
                                None,
                                Err(anyhow!("app-server activity stream lagged by {count} events")),
                            );
                            yield Ok(Frame::data(encode_sse_output(&output)));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
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

fn text_response(status: StatusCode, body: String) -> HttpResponse {
    response_with_body(status, "text/plain; charset=utf-8", Bytes::from(body))
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
mod tests;
