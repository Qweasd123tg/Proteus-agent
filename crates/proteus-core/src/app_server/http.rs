use std::{convert::Infallible, path::PathBuf};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use http_body_util::{BodyExt, Limited, combinators::UnsyncBoxBody};
use hyper::{
    Method, Request, Response, StatusCode, body::Body, server::conn::http1, service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::{net::TcpListener, sync::broadcast};

use crate::core::{
    AppConfig, render_topology_map, render_topology_mermaid, render_topology_runtime_mermaid,
    render_topology_runtime_path,
};

use super::{
    AgentAppServer, AppServerEvent, AppServerHandle, AppSessionActivity, protocol::StdioRequest,
};

mod commands;
mod config;
mod requests;
mod responses;
mod security;
mod sessions;
mod sse;
mod state;

#[cfg(test)]
use commands::spawn_send_turn;
use commands::{
    command_response, execute_app_request, execute_delete_session, execute_new_session,
    execute_resume, execute_send, execute_send_async, execute_set_model,
    execute_set_permission_mode, execute_set_reasoning_effort, execute_set_reasoning_enabled,
    execute_set_web_config,
};
pub use config::HttpServerConfig;
use requests::{
    ApprovalRequest, CancelRequest, DeleteSessionRequest, NewSessionRequest, ResumeSessionRequest,
    SendRequest, SetConfigBuilderRequest, SetModelRequest, SetPermissionModeRequest,
    SetReasoningEffortRequest, SetReasoningEnabledRequest, SetWebConfigRequest, UserInputRequest,
};
use responses::{add_cors_headers, error_response, json_response, options_response, text_response};
use security::{
    HttpSecurity, request_has_valid_token, request_requires_session_token, validate_origin,
};
use sessions::{context_map_json, history_json, session_summaries_json};
use sse::sse_response;
use state::HttpAppState;

#[cfg(test)]
use super::protocol::StdioOutput;
#[cfg(test)]
use http_body_util::Full;
#[cfg(test)]
use hyper::header::CONTENT_TYPE;
#[cfg(test)]
use sse::encode_sse_output;
#[cfg(test)]
use state::RunningTurn;

type HttpBody = UnsyncBoxBody<Bytes, Infallible>;
type HttpResponse = Response<HttpBody>;

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
                    if let Err(error) = http1::Builder::new().serve_connection(io, service).await
                        && should_log_http_connection_error(&error)
                    {
                        eprintln!("app-server HTTP connection error: {error}");
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

#[cfg(test)]
mod tests;
