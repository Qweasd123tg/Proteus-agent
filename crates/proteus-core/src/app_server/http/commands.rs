use std::path::PathBuf;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::{
    app_server::{
        AgentAppServer, AppServerHandle,
        protocol::{StdioOutput, StdioRequest},
    },
    contracts::CancellationToken,
    core::canonicalize_session_dir_path,
    domain::{AgentOutput, PermissionMode},
};

use super::{
    config::new_request_id,
    sessions::server_for_optional_session,
    state::{HttpAppState, RunningTurn, session_key as canonical_session_key},
};

pub(super) async fn execute_app_request(
    state: &HttpAppState,
    request: StdioRequest,
) -> StdioOutput {
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

pub(super) async fn execute_send(
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

pub(super) async fn spawn_send_turn(
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

pub(super) async fn execute_send_async(
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

pub(super) async fn execute_set_permission_mode(
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

pub(super) async fn execute_set_model(
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

pub(super) async fn execute_set_web_config(
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

pub(super) async fn execute_set_reasoning_effort(
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

pub(super) async fn execute_set_reasoning_enabled(
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

pub(super) async fn execute_resume(
    state: &HttpAppState,
    id: Option<String>,
    session_dir: PathBuf,
) -> StdioOutput {
    let result = resume_session(state, session_dir).await.map(Some);
    command_response(id, result)
}

pub(super) async fn execute_new_session(state: &HttpAppState, id: Option<String>) -> StdioOutput {
    let result = new_session(state).await.map(Some);
    command_response(id, result)
}

pub(super) async fn execute_delete_session(
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

pub(super) fn command_response(id: Option<String>, result: Result<Option<Value>>) -> StdioOutput {
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
