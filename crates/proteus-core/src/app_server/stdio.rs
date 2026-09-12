use std::path::PathBuf;

use anyhow::Result;
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    sync::mpsc,
};

use crate::{
    contracts::CancellationToken,
    core::{AppConfig, SteeringQueueReceipt},
};

use super::runs::SendDispatch;
use super::{AgentAppServer, AppServerEvent, StdioOutput, StdioRequest};

pub async fn run_stdio_app_server(
    config: AppConfig,
    cwd: PathBuf,
    config_path: Option<PathBuf>,
    resume_session_dir: Option<PathBuf>,
    fresh_session: bool,
) -> Result<()> {
    let server = if let Some(session_dir) = resume_session_dir {
        AgentAppServer::launch_resumed(config, cwd, config_path.as_deref(), session_dir).await?
    } else if fresh_session {
        // Subagent process runner (и любой orchestrating-родитель) запускает
        // ребёнка со свежей session: resume последней workspace session здесь
        // подхватил бы чужую (например, родительскую) историю.
        AgentAppServer::launch(config, cwd, config_path.as_deref()).await?
    } else {
        AgentAppServer::launch_or_resume_latest(config, cwd, config_path.as_deref()).await?
    };
    let (output_tx, mut output_rx) = mpsc::channel::<StdioOutput>(256);

    let mut events = server.subscribe_session();
    let event_tx = output_tx.clone();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let should_stop = matches!(event, AppServerEvent::Shutdown);
                    if event_tx
                        .send(StdioOutput::Event {
                            event: Box::new(event),
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                    if should_stop {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    server.start_session().await?;

    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::BufWriter::new(tokio::io::stdout());
        while let Some(output) = output_rx.recv().await {
            let line = serde_json::to_string(&output)?;
            stdout.write_all(line.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    let mut shutdown_requested = false;

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let request = match serde_json::from_str::<StdioRequest>(&line) {
            Ok(request) => request,
            Err(error) => {
                send_stdio_response(
                    &output_tx,
                    None,
                    Err(anyhow::anyhow!("invalid JSONL request: {error}")),
                )
                .await;
                continue;
            }
        };
        let id = request.id();

        match request {
            StdioRequest::Send { id, text, options } => {
                match server
                    .dispatch_user_message(id.clone(), text, options, CancellationToken::new())
                    .await
                {
                    Ok(SendDispatch::Queued(receipt)) => {
                        send_stdio_response(&output_tx, id, Ok(Some(queued_response(&receipt))))
                            .await;
                    }
                    Ok(SendDispatch::Started(result)) => {
                        let tx = output_tx.clone();
                        tokio::spawn(async move {
                            let result = match result.await {
                                Ok(result) => result.and_then(|output| {
                                    serde_json::to_value(output).map(Some).map_err(Into::into)
                                }),
                                Err(error) => Err(error.into()),
                            };
                            send_stdio_response(&tx, id, result).await;
                        });
                    }
                    Err(error) => send_stdio_response(&output_tx, id, Err(error)).await,
                }
            }
            StdioRequest::EditQueuedMessage {
                message_id, text, ..
            } => {
                send_stdio_response(
                    &output_tx,
                    id,
                    server
                        .edit_queued_user_message(message_id, text)
                        .await
                        .map(|_| None),
                )
                .await;
            }
            StdioRequest::DeleteQueuedMessage { message_id, .. } => {
                send_stdio_response(
                    &output_tx,
                    id,
                    server
                        .delete_queued_user_message(message_id)
                        .await
                        .map(|_| None),
                )
                .await;
            }
            StdioRequest::ClearHistory { .. } => {
                send_stdio_response(&output_tx, id, server.clear_history().await.map(|_| None))
                    .await;
            }
            StdioRequest::UsageSummary { .. } => {
                let result = match server.usage_snapshot().await {
                    Ok(snapshot) => serde_json::to_value(snapshot).map(Some).map_err(Into::into),
                    Err(error) => Err(error),
                };
                send_stdio_response(&output_tx, id, result).await;
            }
            StdioRequest::HistorySummary { .. } => {
                let result = serde_json::to_value(server.history_summary().await)
                    .map(Some)
                    .map_err(anyhow::Error::from);
                send_stdio_response(&output_tx, id, result).await;
            }
            StdioRequest::Remember { kind, content, .. } => {
                let result = server.remember(kind, content).await.and_then(|result| {
                    serde_json::to_value(result)
                        .map(Some)
                        .map_err(anyhow::Error::from)
                });
                send_stdio_response(&output_tx, id, result).await;
            }
            StdioRequest::Approval {
                approval_id,
                approved,
                note,
                cache,
                ..
            } => {
                send_stdio_response(
                    &output_tx,
                    id,
                    server
                        .respond_approval(&approval_id, approved, note, cache)
                        .await
                        .map(|_| None),
                )
                .await;
            }
            StdioRequest::UserInput {
                request_id,
                response,
                ..
            } => {
                send_stdio_response(
                    &output_tx,
                    id,
                    server
                        .respond_user_input(&request_id, response)
                        .await
                        .map(|_| None),
                )
                .await;
            }
            StdioRequest::Cancel { target_id, .. } => {
                let result = server.cancel_run(&target_id).await;
                send_stdio_response(&output_tx, id, result.map(|_| None)).await;
            }
            StdioRequest::SetPermissionMode { mode, .. } => {
                server.set_permission_mode(mode).await;
                send_stdio_response(
                    &output_tx,
                    id,
                    Ok(Some(serde_json::json!({ "mode": mode }))),
                )
                .await;
            }
            StdioRequest::SetModel { model, .. } => {
                let result = server.set_model_name(model.clone()).await;
                send_stdio_response(
                    &output_tx,
                    id,
                    match result {
                        Ok(()) => Ok(Some(serde_json::json!({ "model": model, "config": server.config_summary().await }))),
                        Err(error) => Err(error),
                    },
                )
                .await;
            }
            StdioRequest::SetReasoningEffort { effort, .. } => {
                let result = server.set_reasoning_effort(effort.clone()).await;
                send_stdio_response(
                    &output_tx,
                    id,
                    result.map(|_| Some(serde_json::json!({ "effort": effort }))),
                )
                .await;
            }
            StdioRequest::SetReasoningEnabled { enabled, .. } => {
                server.set_reasoning_enabled(enabled).await;
                send_stdio_response(
                    &output_tx,
                    id,
                    Ok(Some(serde_json::json!({ "enabled": enabled }))),
                )
                .await;
            }
            StdioRequest::ConfigSummary { .. } => {
                send_stdio_response(&output_tx, id, Ok(Some(server.config_summary().await))).await;
            }
            StdioRequest::ReloadTools { .. } => {
                let result = server.reload_tools().await.and_then(|report| {
                    serde_json::to_value(report)
                        .map(Some)
                        .map_err(anyhow::Error::from)
                });
                send_stdio_response(&output_tx, id, result).await;
            }
            StdioRequest::Shutdown { .. } => {
                shutdown_requested = true;
                server.shutdown().await;
                send_stdio_response(&output_tx, id, Ok(None)).await;
                break;
            }
        }
    }

    if !shutdown_requested {
        server.shutdown().await;
    }
    drop(output_tx);
    writer.await??;
    Ok(())
}

fn queued_response(receipt: &SteeringQueueReceipt) -> Value {
    serde_json::json!({
        "accepted": true,
        "queued": true,
        "message_id": receipt.message_id,
        "active_turn_id": receipt.active_turn_id,
        "queued_count": receipt.queued_count,
    })
}

async fn send_stdio_response(
    output_tx: &mpsc::Sender<StdioOutput>,
    id: Option<String>,
    result: Result<Option<Value>>,
) {
    let output = match result {
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
    };
    let _ = output_tx.send(output).await;
}
