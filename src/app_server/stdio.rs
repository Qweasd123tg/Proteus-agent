use std::{collections::HashMap, path::PathBuf};

use anyhow::{Result, anyhow};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    sync::mpsc,
};

use crate::core::AppConfig;

use super::{
    AgentAppServer, AppServerEvent, AppServerHandle,
    protocol::{StdioOutput, StdioRequest},
};

pub async fn run_stdio_app_server(
    config: AppConfig,
    cwd: PathBuf,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let server = AgentAppServer::launch(config, cwd, config_path.as_deref())?;
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
                        .send(StdioOutput::Response {
                            id: None,
                            ok: false,
                            output: None,
                            error: Some(format!(
                                "app-server event stream lagged by {count} events"
                            )),
                        })
                        .await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

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
    let mut keyed_turn_handles = HashMap::new();
    let mut anonymous_turn_handles = Vec::new();
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
                prune_finished_turns(&mut keyed_turn_handles);
                match id.clone() {
                    Some(turn_id) if keyed_turn_handles.contains_key(&turn_id) => {
                        send_stdio_response(
                            &output_tx,
                            id,
                            Err(anyhow!("turn id is already running: {turn_id}")),
                        )
                        .await;
                    }
                    Some(turn_id) => {
                        keyed_turn_handles.insert(
                            turn_id,
                            spawn_stdio_turn(server.clone(), output_tx.clone(), id, text),
                        );
                    }
                    None => {
                        anonymous_turn_handles.push(spawn_stdio_turn(
                            server.clone(),
                            output_tx.clone(),
                            None,
                            text,
                        ));
                    }
                }
            }
            StdioRequest::ClearHistory { .. } => {
                send_stdio_response(&output_tx, id, server.clear_history().await.map(|_| None))
                    .await;
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
            StdioRequest::Cancel { target_id, .. } => {
                let result =
                    cancel_stdio_turn(&server, &mut keyed_turn_handles, &output_tx, &target_id)
                        .await;
                send_stdio_response(&output_tx, id, result.map(|_| None)).await;
            }
            StdioRequest::Shutdown { .. } => {
                shutdown_requested = true;
                server.shutdown().await;
                send_stdio_response(&output_tx, id, Ok(None)).await;
                break;
            }
        }
    }

    if shutdown_requested {
        for (_, handle) in keyed_turn_handles {
            handle.abort();
        }
        for handle in anonymous_turn_handles {
            handle.abort();
        }
    } else {
        for (_, handle) in keyed_turn_handles {
            let _ = handle.await;
        }
        for handle in anonymous_turn_handles {
            let _ = handle.await;
        }
        server.shutdown().await;
    }
    drop(output_tx);
    writer.await??;
    Ok(())
}

fn spawn_stdio_turn(
    server: AppServerHandle,
    output_tx: mpsc::Sender<StdioOutput>,
    id: Option<String>,
    text: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let result = match server.send_user_message(text).await {
            Ok(output) => serde_json::to_value(output)
                .map(Some)
                .map_err(anyhow::Error::from),
            Err(error) => Err(error),
        };
        send_stdio_response(&output_tx, id, result).await;
    })
}

async fn cancel_stdio_turn(
    server: &AppServerHandle,
    turn_handles: &mut HashMap<String, tokio::task::JoinHandle<()>>,
    output_tx: &mpsc::Sender<StdioOutput>,
    target_id: &str,
) -> Result<()> {
    prune_finished_turns(turn_handles);
    let handle = turn_handles
        .remove(target_id)
        .ok_or_else(|| anyhow!("unknown or completed turn id: {target_id}"))?;
    handle.abort();
    send_stdio_response(
        output_tx,
        Some(target_id.to_owned()),
        Err(anyhow!("turn canceled by client")),
    )
    .await;
    server
        .cancel_pending_approvals("turn canceled by client".to_owned())
        .await;
    Ok(())
}

fn prune_finished_turns(turn_handles: &mut HashMap<String, tokio::task::JoinHandle<()>>) {
    turn_handles.retain(|_, handle| !handle.is_finished());
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
    use std::{collections::HashMap, time::Duration};

    use tokio::sync::mpsc;

    use super::*;

    #[tokio::test]
    async fn cancel_stdio_turn_aborts_handle_and_sends_target_error_response() {
        let cwd = tempfile::tempdir().expect("cwd");
        let server = AgentAppServer::launch(AppConfig::default(), cwd.path().to_path_buf(), None)
            .expect("app server");
        let (output_tx, mut output_rx) = mpsc::channel(4);
        let mut turn_handles = HashMap::new();
        turn_handles.insert(
            "send-1".to_owned(),
            tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }),
        );

        cancel_stdio_turn(&server, &mut turn_handles, &output_tx, "send-1")
            .await
            .expect("cancel turn");

        assert!(turn_handles.is_empty());
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
                assert_eq!(error.as_deref(), Some("turn canceled by client"));
            }
            StdioOutput::Event { .. } => panic!("expected response"),
        }
        server.shutdown().await;
    }
}
