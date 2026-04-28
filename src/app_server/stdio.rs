use std::path::PathBuf;

use anyhow::Result;
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
    let mut turn_handles = Vec::new();
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
                turn_handles.push(spawn_stdio_turn(
                    server.clone(),
                    output_tx.clone(),
                    id,
                    text,
                ));
            }
            StdioRequest::ClearHistory { .. } => {
                send_stdio_response(&output_tx, id, server.clear_history().await.map(|_| None))
                    .await;
            }
            StdioRequest::Approval {
                approval_id,
                approved,
                note,
                ..
            } => {
                send_stdio_response(
                    &output_tx,
                    id,
                    server
                        .respond_approval(&approval_id, approved, note)
                        .await
                        .map(|_| None),
                )
                .await;
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
        for handle in turn_handles {
            handle.abort();
        }
    } else {
        for handle in turn_handles {
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
