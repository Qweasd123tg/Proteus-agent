use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, anyhow};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    sync::mpsc,
};

use crate::{
    contracts::CancellationToken,
    core::{AppConfig, ReservedUserMessage, SteeringQueueReceipt, UserMessageReservation},
};

use super::{AgentAppServer, AppServerEvent, AppServerHandle, StdioOutput, StdioRequest};

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

    let mut events = server.subscribe();
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
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    let _ = event_tx
                        .send(StdioOutput::Event {
                            event: Box::new(AppServerEvent::EventStreamLagged { count }),
                        })
                        .await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
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
    let mut keyed_run_handles = HashMap::<String, StdioRunHandle>::new();
    let mut anonymous_run_handles = Vec::<StdioRunHandle>::new();
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
            StdioRequest::Send { id, text } => {
                prune_finished_runs(&mut keyed_run_handles);
                if let Some(run_id) = id.clone()
                    && keyed_run_handles.contains_key(&run_id)
                {
                    send_stdio_response(
                        &output_tx,
                        id,
                        Err(anyhow!("run id is already active: {run_id}")),
                    )
                    .await;
                    continue;
                }
                match server.reserve_user_message(text).await {
                    Ok(UserMessageReservation::Queued(receipt)) => {
                        send_stdio_response(&output_tx, id, Ok(Some(queued_response(&receipt))))
                            .await;
                    }
                    Ok(UserMessageReservation::Start(reserved)) => match id.clone() {
                        Some(run_id) => {
                            keyed_run_handles.insert(
                                run_id,
                                spawn_stdio_run(server.clone(), output_tx.clone(), id, reserved),
                            );
                        }
                        None => anonymous_run_handles.push(spawn_stdio_run(
                            server.clone(),
                            output_tx.clone(),
                            None,
                            reserved,
                        )),
                    },
                    Err(error) => send_stdio_response(&output_tx, id, Err(error)).await,
                }
            }
            StdioRequest::ClearHistory { .. } => {
                send_stdio_response(&output_tx, id, server.clear_history().await.map(|_| None))
                    .await;
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
                let result = cancel_stdio_run(&mut keyed_run_handles, &output_tx, &target_id).await;
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
                server.set_model_name(model.clone()).await;
                send_stdio_response(
                    &output_tx,
                    id,
                    Ok(Some(serde_json::json!({ "model": model }))),
                )
                .await;
            }
            StdioRequest::SetReasoningEffort { effort, .. } => {
                server.set_reasoning_effort(effort.clone()).await;
                send_stdio_response(
                    &output_tx,
                    id,
                    Ok(Some(serde_json::json!({ "effort": effort }))),
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
            _ => {
                send_stdio_response(
                    &output_tx,
                    id,
                    Err(anyhow!("unsupported StdioRequest variant")),
                )
                .await;
            }
        }
    }

    if !shutdown_requested {
        server.shutdown().await;
    }
    cancel_and_join_stdio_runs(keyed_run_handles, anonymous_run_handles).await;
    drop(output_tx);
    writer.await??;
    Ok(())
}

fn spawn_stdio_run(
    server: AppServerHandle,
    output_tx: mpsc::Sender<StdioOutput>,
    id: Option<String>,
    reserved: ReservedUserMessage,
) -> StdioRunHandle {
    let cancellation = CancellationToken::new();
    let run_cancellation = cancellation.clone();
    let response_claimed = Arc::new(AtomicBool::new(false));
    let task_response_claimed = response_claimed.clone();
    let join = tokio::spawn(async move {
        let result = match server
            .run_reserved_user_message(reserved, run_cancellation)
            .await
        {
            Ok(output) => serde_json::to_value(output)
                .map(Some)
                .map_err(anyhow::Error::from),
            Err(error) => Err(error),
        };
        if !task_response_claimed.swap(true, Ordering::AcqRel) {
            send_stdio_response(&output_tx, id, result).await;
        }
    });
    StdioRunHandle {
        join,
        cancellation,
        response_claimed,
    }
}

struct StdioRunHandle {
    join: tokio::task::JoinHandle<()>,
    cancellation: CancellationToken,
    response_claimed: Arc<AtomicBool>,
}

impl StdioRunHandle {
    fn claim_response(&self) -> bool {
        !self.response_claimed.swap(true, Ordering::AcqRel)
    }

    async fn cancel_and_join(mut self) {
        self.cancellation.cancel();
        if tokio::time::timeout(Duration::from_secs(1), &mut self.join)
            .await
            .is_err()
        {
            self.join.abort();
            let _ = self.join.await;
        }
    }
}

async fn cancel_stdio_run(
    run_handles: &mut HashMap<String, StdioRunHandle>,
    output_tx: &mpsc::Sender<StdioOutput>,
    target_id: &str,
) -> Result<()> {
    prune_finished_runs(run_handles);
    let handle = run_handles
        .remove(target_id)
        .ok_or_else(|| anyhow!("unknown or completed run id: {target_id}"))?;
    let should_send_target_response = handle.claim_response();
    handle.cancel_and_join().await;
    if should_send_target_response {
        send_stdio_response(
            output_tx,
            Some(target_id.to_owned()),
            Err(anyhow!("run canceled by client")),
        )
        .await;
    }
    // Pending approvals и user inputs отменённого turn-а резолвятся
    // watcher-ами app-server-а, когда orchestrator дропает свои futures:
    // blanket-deny здесь затрагивал бы pending запросы других конкурентных
    // turn-ов.
    Ok(())
}

fn prune_finished_runs(run_handles: &mut HashMap<String, StdioRunHandle>) {
    run_handles.retain(|_, handle| !handle.join.is_finished());
}

async fn cancel_and_join_stdio_runs(
    keyed_run_handles: HashMap<String, StdioRunHandle>,
    anonymous_run_handles: Vec<StdioRunHandle>,
) {
    for (_, handle) in keyed_run_handles {
        handle.claim_response();
        handle.cancel_and_join().await;
    }
    for handle in anonymous_run_handles {
        handle.claim_response();
        handle.cancel_and_join().await;
    }
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tokio::sync::mpsc;

    use super::*;

    #[tokio::test]
    async fn cancel_stdio_run_joins_handle_and_sends_target_error_response() {
        let (output_tx, mut output_rx) = mpsc::channel(4);
        let mut run_handles = HashMap::new();
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        run_handles.insert(
            "send-1".to_owned(),
            StdioRunHandle {
                join: tokio::spawn(async move {
                    task_cancellation.cancelled().await;
                }),
                cancellation: cancellation.clone(),
                response_claimed: Arc::new(AtomicBool::new(false)),
            },
        );

        cancel_stdio_run(&mut run_handles, &output_tx, "send-1")
            .await
            .expect("cancel run");

        assert!(run_handles.is_empty());
        assert!(cancellation.is_cancelled());
        let output = output_rx.recv().await.expect("target response");
        match output {
            StdioOutput::Response {
                id,
                ok,
                output,
                error,
            } => {
                assert_eq!(id.as_deref(), Some("send-1"));
                assert!(!ok);
                assert!(output.is_none());
                assert_eq!(error.as_deref(), Some("run canceled by client"));
            }
            StdioOutput::Event { .. } => panic!("expected response"),
            _ => panic!("unexpected output variant"),
        }
        assert!(
            output_rx.try_recv().is_err(),
            "target response must be unique"
        );
    }
}
