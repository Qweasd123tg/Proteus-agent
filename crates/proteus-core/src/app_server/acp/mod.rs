//! ACP v1 editor transport. Runtime admission, tools and policy remain session-owned.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
};

use agent_client_protocol::schema::{ProtocolVersion, v1::*};
use agent_client_protocol::{Agent, Client, ConnectTo, ConnectionTo, Error, Result, Stdio};
use tokio::sync::Mutex;

use super::{AgentAppServer, AppServerHandle};
use crate::{contracts::CancellationToken, core::AppConfig};

mod input;
mod projection;
mod prompt;

struct State {
    config: AppConfig,
    config_path: Option<PathBuf>,
    initialized: Mutex<bool>,
    sessions: Mutex<HashMap<SessionId, Session>>,
}

#[derive(Clone)]
struct Session {
    server: AppServerHandle,
    active: Arc<StdMutex<Option<CancellationToken>>>,
}

/// Covers the protocol prompt lifetime, including delivery after runtime settlement.
struct PromptLease {
    active: Arc<StdMutex<Option<CancellationToken>>>,
    cancellation: CancellationToken,
}

impl Drop for PromptLease {
    fn drop(&mut self) {
        self.active.lock().unwrap().take();
    }
}

impl Session {
    fn reserve(&self) -> Result<PromptLease> {
        let mut active = self.active.lock().unwrap();
        if active.is_some() {
            return Err(invalid("session already has an active ACP prompt"));
        }
        let cancellation = CancellationToken::new();
        *active = Some(cancellation.clone());
        Ok(PromptLease {
            active: self.active.clone(),
            cancellation,
        })
    }
}

/// Serve ACP on stdin/stdout; diagnostics and child stderr never enter stdout.
pub async fn run_acp_server(config: AppConfig, config_path: Option<PathBuf>) -> anyhow::Result<()> {
    serve(config, config_path, Stdio::new())
        .await
        .map_err(Into::into)
}

async fn serve(
    config: AppConfig,
    config_path: Option<PathBuf>,
    transport: impl ConnectTo<Agent>,
) -> Result<()> {
    let state = Arc::new(State {
        config,
        config_path,
        initialized: Mutex::new(false),
        sessions: Mutex::new(HashMap::new()),
    });
    let initialize = state.clone();
    let new_session = state.clone();
    let prompt = state.clone();
    let cancel = state.clone();
    let mode = state.clone();
    let close = state.clone();
    let result = Agent
        .builder()
        .name("proteus")
        .on_receive_request(
            async move |_: InitializeRequest, responder, _| {
                let mut initialized = initialize.initialized.lock().await;
                if *initialized {
                    return responder
                        .respond_with_error(invalid("connection is already initialized"));
                }
                *initialized = true;
                responder.respond(
                    InitializeResponse::new(ProtocolVersion::V1)
                        .agent_info(Implementation::new("proteus", env!("CARGO_PKG_VERSION")))
                        .agent_capabilities(AgentCapabilities::new()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _| {
                responder.respond_with_result(new_session.new_session(request).await)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                // Reserve before returning to dispatch: a following cancel must see this run.
                match prompt.session(&request.session_id).await {
                    Ok(session) => {
                        let lease = match session.reserve() {
                            Ok(lease) => lease,
                            Err(error) => return responder.respond_with_error(error),
                        };
                        match prompt::prepare(session.server, request, lease.cancellation.clone())
                            .await
                        {
                            Ok(run) => {
                                let connection = cx.clone();
                                cx.spawn(async move {
                                    let _lease = lease;
                                    responder.respond_with_result(run.finish(connection).await)
                                })
                            }
                            Err(error) => responder.respond_with_error(error),
                        }
                    }
                    Err(error) => responder.respond_with_error(error),
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |request: CancelNotification, _| {
                if let Ok(session) = cancel.session(&request.session_id).await {
                    if let Some(token) = session.active.lock().unwrap().as_ref() {
                        token.cancel();
                    }
                    for id in session.server.running_run_ids().await {
                        let _ = session.server.cancel_run(&id).await;
                    }
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: SetSessionModeRequest, responder, cx| {
                let result = async {
                    let session = mode.session(&request.session_id).await?;
                    if session.active.lock().unwrap().is_some() {
                        return Err(invalid("cannot change mode during an active prompt"));
                    }
                    let permission = input::permission_mode(&request.mode_id)?;
                    session.server.set_permission_mode(permission).await;
                    cx.send_notification(SessionNotification::new(
                        request.session_id,
                        SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(request.mode_id)),
                    ))?;
                    Ok(SetSessionModeResponse::new())
                }
                .await;
                responder.respond_with_result(result)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_close(async move |_| {
            close.shutdown().await;
            Ok(())
        })
        .connect_to(transport)
        .await;
    // on_close covers clean EOF. Transport/dispatch errors need the same settlement.
    state.shutdown().await;
    result
}

impl State {
    async fn session(&self, id: &SessionId) -> Result<Session> {
        self.ensure_initialized().await?;
        self.sessions
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| invalid("unknown sessionId"))
    }

    async fn ensure_initialized(&self) -> Result<()> {
        if !*self.initialized.lock().await {
            return Err(invalid("initialize must precede session methods"));
        }
        Ok(())
    }

    async fn new_session(&self, request: NewSessionRequest) -> Result<NewSessionResponse> {
        self.ensure_initialized().await?;
        if !request.cwd.is_absolute() || !request.cwd.is_dir() {
            return Err(invalid("cwd must be an existing absolute directory"));
        }
        let config = input::session_config(self.config.clone(), request.mcp_servers)?;
        let server = AgentAppServer::launch(config, request.cwd, self.config_path.as_deref())
            .await
            .map_err(internal)?;
        if let Err(error) = server.start_session().await {
            server.shutdown().await;
            return Err(internal(error));
        }
        let modes = input::modes(server.permission_mode().await)?;
        let id = SessionId::new(server.session_id().to_string());
        self.sessions.lock().await.insert(
            id.clone(),
            Session {
                server,
                active: Arc::new(StdMutex::new(None)),
            },
        );
        Ok(NewSessionResponse::new(id).modes(modes))
    }

    async fn shutdown(&self) {
        let sessions = std::mem::take(&mut *self.sessions.lock().await);
        // Start cancellation for every session before waiting on any single one.
        futures_util::future::join_all(sessions.values().map(|s| s.server.shutdown())).await;
    }
}

fn invalid(message: impl ToString) -> Error {
    Error::invalid_params().data(serde_json::json!(message.to_string()))
}

fn internal(error: impl std::fmt::Display) -> Error {
    Error::internal_error().data(serde_json::json!(error.to_string()))
}

#[cfg(test)]
mod tests;
