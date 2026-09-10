use std::path::PathBuf;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::{
    app_server::{AppServerHandle, StdioOutput, StdioRequest},
    contracts::CancellationToken,
    core::{SteeringQueueReceipt, UserMessageReservation},
    domain::PermissionMode,
};

use super::{
    config::new_request_id,
    sessions::server_for_session,
    state::{HttpAppState, RunningRun, session_key as canonical_session_key},
};

pub(super) async fn execute_app_request(
    state: &HttpAppState,
    request: StdioRequest,
    query: Option<&str>,
) -> StdioOutput {
    if matches!(request, StdioRequest::Shutdown { .. }) {
        shutdown_all_servers(state).await;
        let _ = state.shutdown.send(());
        return command_response(request.id(), Ok(None));
    }
    match super::sessions::server_for_query(state, query).await {
        Ok(server) => execute_session_request(state, &server, request).await,
        Err(error) => command_response(request.id(), Err(error)),
    }
}

async fn execute_session_request(
    state: &HttpAppState,
    server: &AppServerHandle,
    request: StdioRequest,
) -> StdioOutput {
    let id = request.id();
    let result = match request {
        StdioRequest::Send { id, text } => execute_send(
            state,
            id,
            text,
            server.session_dir_path().expect("addressed session"),
        )
        .await
        .map(Some),
        StdioRequest::UsageSummary { .. } => match server.usage_snapshot().await {
            Ok(snapshot) => serde_json::to_value(snapshot).map(Some).map_err(Into::into),
            Err(error) => Err(error),
        },
        StdioRequest::EditQueuedMessage {
            message_id, text, ..
        } => server
            .edit_queued_user_message(message_id, text)
            .await
            .map(|_| None),
        StdioRequest::DeleteQueuedMessage { message_id, .. } => {
            let server = server.clone();
            let result = server
                .delete_queued_user_message(message_id)
                .await
                .map(|_| None);
            state.emit_session_activity_for_server(&server).await;
            result
        }
        StdioRequest::ClearHistory { .. } => server.clear_history().await.map(|_| None),
        StdioRequest::HistorySummary { .. } => serde_json::to_value(server.history_summary().await)
            .map(Some)
            .map_err(anyhow::Error::from),
        StdioRequest::Remember { kind, content, .. } => {
            server.remember(kind, content).await.and_then(|result| {
                serde_json::to_value(result)
                    .map(Some)
                    .map_err(anyhow::Error::from)
            })
        }
        StdioRequest::Approval {
            approval_id,
            approved,
            note,
            cache,
            ..
        } => {
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
            let result = server
                .respond_user_input(&request_id, response)
                .await
                .map(|_| None);
            state.emit_session_activity_for_server(&server).await;
            result
        }
        StdioRequest::Cancel { target_id, .. } => execute_cancel(state, server, &target_id)
            .await
            .map(|_| None),
        StdioRequest::SetPermissionMode { mode, .. } => {
            server.set_permission_mode(mode).await;
            Ok(Some(json!({ "mode": mode })))
        }
        StdioRequest::SetModel { model, .. } => {
            let server = server.clone();
            match server.set_model_name(model.clone()).await {
                Ok(()) => Ok(Some(
                    json!({ "model": model, "config": server.config_summary().await }),
                )),
                Err(error) => Err(error),
            }
        }
        StdioRequest::SetReasoningEffort { effort, .. } => server
            .set_reasoning_effort(effort.clone())
            .await
            .map(|_| Some(json!({ "effort": effort }))),
        StdioRequest::SetReasoningEnabled { enabled, .. } => {
            server.set_reasoning_enabled(enabled).await;
            Ok(Some(json!({ "enabled": enabled })))
        }
        StdioRequest::ConfigSummary { .. } => Ok(Some(server.config_summary().await)),
        StdioRequest::ReloadTools { .. } => server.reload_tools().await.and_then(|report| {
            serde_json::to_value(report)
                .map(Some)
                .map_err(anyhow::Error::from)
        }),
        _ => Err(anyhow!("unsupported StdioRequest variant")),
    };
    command_response(id, result)
}

pub(super) async fn execute_send(
    state: &HttpAppState,
    id: Option<String>,
    text: String,
    session_dir: PathBuf,
) -> Result<Value> {
    let cancellation = CancellationToken::new();
    let server = server_for_session(state, session_dir).await?;
    match spawn_send_run(state, server, id, text, cancellation).await? {
        SendDispatch::Started(receiver) => {
            let output = receiver
                .await
                .map_err(|_| anyhow!("send run task dropped before completion"))??;
            serde_json::to_value(output).map_err(anyhow::Error::from)
        }
        SendDispatch::Queued(receipt) => Ok(queued_response(&receipt)),
    }
}

pub(super) enum SendDispatch {
    Started(oneshot::Receiver<Result<crate::domain::AgentOutput>>),
    Queued(SteeringQueueReceipt),
}

pub(super) async fn spawn_send_run(
    state: &HttpAppState,
    server: AppServerHandle,
    run_id: Option<String>,
    text: String,
    cancellation: CancellationToken,
) -> Result<SendDispatch> {
    let session_dir = server.session_dir_path();
    let mut running_runs = state.running_runs.lock().await;
    if let Some(run_id) = run_id.as_deref()
        && running_runs.contains_key(run_id)
    {
        return Err(anyhow!("run id is already active: {run_id}"));
    }
    let reservation = server.reserve_user_message(text).await?;
    let reserved = match reservation {
        UserMessageReservation::Queued(receipt) => {
            drop(running_runs);
            state.emit_session_activity_for_server(&server).await;
            return Ok(SendDispatch::Queued(receipt));
        }
        UserMessageReservation::Start(reserved) => reserved,
    };
    if let Some(run_id) = run_id.as_deref() {
        running_runs.insert(
            run_id.to_owned(),
            RunningRun::new(cancellation.clone(), session_dir.clone()),
        );
    }
    drop(running_runs);
    state.emit_session_activity_for_server(&server).await;

    let (result_tx, result_rx) = oneshot::channel();
    let state_for_activity = state.clone();
    tokio::spawn(async move {
        let result = server
            .run_reserved_user_message(reserved, cancellation)
            .await;
        if let Some(run_id) = run_id.as_deref() {
            state_for_activity.running_runs.lock().await.remove(run_id);
        }
        state_for_activity
            .emit_session_activity_for_server(&server)
            .await;
        let _ = result_tx.send(result);
    });
    Ok(SendDispatch::Started(result_rx))
}

pub(super) async fn execute_send_async(
    state: &HttpAppState,
    id: Option<String>,
    text: String,
    session_dir: PathBuf,
) -> StdioOutput {
    let run_id = id.unwrap_or_else(new_request_id);
    let cancellation = CancellationToken::new();
    let server = match server_for_session(state, session_dir).await {
        Ok(server) => server,
        Err(error) => return command_response(Some(run_id), Err(error)),
    };
    match spawn_send_run(state, server, Some(run_id.clone()), text, cancellation).await {
        Ok(SendDispatch::Started(_)) => command_response(
            Some(run_id.clone()),
            Ok(Some(json!({
                "run_id": run_id,
                "accepted": true,
                "queued": false,
            }))),
        ),
        Ok(SendDispatch::Queued(receipt)) => command_response(
            Some(run_id.clone()),
            Ok(Some(queued_response_with_request_id(&receipt, &run_id))),
        ),
        Err(error) => command_response(Some(run_id), Err(error)),
    }
}

fn queued_response(receipt: &SteeringQueueReceipt) -> Value {
    json!({
        "accepted": true,
        "queued": true,
        "message_id": receipt.message_id,
        "active_turn_id": receipt.active_turn_id,
        "queued_count": receipt.queued_count,
    })
}

fn queued_response_with_request_id(receipt: &SteeringQueueReceipt, request_id: &str) -> Value {
    let mut response = queued_response(receipt);
    response["request_id"] = Value::String(request_id.to_owned());
    response
}

pub(super) async fn execute_set_permission_mode(
    state: &HttpAppState,
    id: Option<String>,
    mode: PermissionMode,
    session_dir: PathBuf,
) -> StdioOutput {
    let result = async {
        let server = server_for_session(state, session_dir).await?;
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
    session_dir: PathBuf,
) -> StdioOutput {
    let result = async {
        let server = server_for_session(state, session_dir).await?;
        server.set_model_name(model.clone()).await?;
        Ok(Some(
            json!({ "model": model, "config": server.config_summary().await }),
        ))
    }
    .await;
    command_response(id, result)
}

pub(super) async fn execute_set_web_config(
    state: &HttpAppState,
    id: Option<String>,
    tool_cards_collapsed: Option<bool>,
    session_dir: PathBuf,
) -> StdioOutput {
    let result = async {
        let server = server_for_session(state, session_dir).await?;
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
    session_dir: PathBuf,
) -> StdioOutput {
    let result = async {
        let server = server_for_session(state, session_dir).await?;
        server.set_reasoning_effort(effort.clone()).await?;
        Ok(Some(json!({ "effort": effort })))
    }
    .await;
    command_response(id, result)
}

pub(super) async fn execute_set_reasoning_enabled(
    state: &HttpAppState,
    id: Option<String>,
    enabled: bool,
    session_dir: PathBuf,
) -> StdioOutput {
    let result = async {
        let server = server_for_session(state, session_dir).await?;
        server.set_reasoning_enabled(enabled).await;
        Ok(Some(json!({ "enabled": enabled })))
    }
    .await;
    command_response(id, result)
}

pub(super) async fn cancel_work_for_server(state: &HttpAppState, server: &AppServerHandle) {
    let session_dir = server.session_dir_path().map(canonical_session_key);
    let active_runs = {
        let mut running_runs = state.running_runs.lock().await;
        let run_ids = running_runs
            .iter()
            .filter_map(|(run_id, run)| {
                if run.session_dir == session_dir {
                    Some(run_id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        run_ids
            .into_iter()
            .filter_map(|run_id| running_runs.remove(&run_id))
            .map(|run| run.cancellation)
            .collect::<Vec<_>>()
    };
    for cancellation in active_runs {
        cancellation.cancel();
    }

    // Pending approvals и user inputs отменяемых turn-ов резолвятся
    // watcher-ами app-server-а после того, как orchestrator дропнет свои
    // futures.
    state.emit_session_activity_for_server(server).await;
}

pub(super) async fn shutdown_all_servers(state: &HttpAppState) {
    let cancellations = {
        let mut running_runs = state.running_runs.lock().await;
        running_runs
            .drain()
            .map(|(_, run)| run.cancellation)
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

async fn execute_cancel(
    state: &HttpAppState,
    server: &AppServerHandle,
    target_id: &str,
) -> Result<()> {
    let session_dir = server.session_dir_path().map(canonical_session_key);
    let run = {
        let mut runs = state.running_runs.lock().await;
        let run = runs
            .get(target_id)
            .filter(|run| run.session_dir == session_dir)
            .ok_or_else(|| anyhow!("unknown or completed run id for session: {target_id}"))?;
        let run = run.clone();
        runs.remove(target_id);
        run
    };
    run.cancellation.cancel();
    // Completion semantics are unchanged; this acknowledges the cancellation request.
    state.emit_session_activity_for_server(server).await;
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
