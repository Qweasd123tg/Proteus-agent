//! Accepted Responses SSE failures are retried by the Codex workflow as new
//! canonical model exchanges. Durable tool effects and completed assistant
//! items are carried into the retry without replaying unfinished stream data.
use std::{path::Path, sync::Arc, time::Duration};

use proteus_contracts::model_standard::{ContentPart, MessagePhase, ModelFailureKind};
use proteus_core::core::{
    AgentRuntime, JournalEntry, ModelResponseOutcome, ModuleCatalog, SessionStore,
    TurnSettlementStatus, WorkflowReplayOptions, replay_workflow,
};
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, net::TcpListener};

use super::{
    direct_tool_surface::{ApprovingTransport, fixture_config},
    read_json_request, sse_body,
};

#[path = "stream_recovery/cancellation.rs"]
mod cancellation;

const PROMPT: &str = "Измени файл и закончи работу после восстановления потока.";
const CALL_ID: &str = "call_before_stream_retry";
const EFFECT_FILE: &str = "sse-retry-effect.log";
const EFFECT: &str = "x";
const COMPLETED: &str = "Изменение выполнено, проверяю результат.";
const UNFINISHED: &str = "этот незавершённый фрагмент нельзя переносить";
const FINAL: &str = "Поток восстановлен без повторного изменения.";
const EXHAUSTED: [&str; 3] = [
    "Завершённый фрагмент первой попытки.",
    "Завершённый фрагмент второй попытки.",
    "Завершённый фрагмент последней попытки.",
];

fn response(output: Value, id: &str) -> Value {
    json!({"id": id, "object": "response", "status": "completed", "output": output})
}

fn tool_response() -> Value {
    response(
        json!([{
            "type": "function_call", "id": "retry_tool_item", "status": "completed",
            "call_id": CALL_ID, "name": "shell",
            "arguments": serde_json::to_string(&json!({
                "command": format!("printf x >> {EFFECT_FILE}; printf 'committed\\n'")
            })).unwrap(),
        }]),
        "retry_tool_response",
    )
}

fn final_response() -> Value {
    response(
        json!([{
            "type": "message", "id": "retry_final_item", "status": "completed",
            "role": "assistant", "phase": "final_answer",
            "content": [{"type": "output_text", "text": FINAL, "annotations": []}],
        }]),
        "retry_final_response",
    )
}

fn partial_sse_body(message_id: &str, text: &str) -> String {
    let completed = json!({
        "type": "message", "id": message_id, "status": "completed",
        "role": "assistant", "phase": "commentary",
        "content": [{"type": "output_text", "text": text, "annotations": []}],
    });
    let unfinished_id = format!("{message_id}_unfinished");
    let unfinished = json!({
        "type": "message", "id": unfinished_id, "status": "in_progress",
        "role": "assistant", "phase": "commentary", "content": [],
    });
    [
        format!(
            "event: response.output_item.done\ndata: {}\n\n",
            json!({"output_index": 0, "item": completed})
        ),
        format!(
            "event: response.output_item.added\ndata: {}\n\n",
            json!({"output_index": 1, "item": unfinished})
        ),
        format!(
            "event: response.output_text.delta\ndata: {}\n\n",
            json!({
                "output_index": 1, "item_id": unfinished_id, "content_index": 0,
                "delta": UNFINISHED,
            })
        ),
    ]
    .concat()
}

async fn write_sse(socket: &mut tokio::net::TcpStream, body: &str) {
    socket
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
}

async fn write_truncated_sse(socket: &mut tokio::net::TcpStream, body: &str) {
    socket
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len() + 128
            )
            .as_bytes(),
        )
        .await
        .unwrap();
}

async fn serve_success(listener: TcpListener) -> Vec<Value> {
    let mut requests = Vec::new();
    for round in 0..3 {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .expect("bounded fixture accept")
            .unwrap();
        requests.push(read_json_request(&mut socket).await);
        match round {
            0 => write_sse(&mut socket, &sse_body(&tool_response())).await,
            1 => {
                write_truncated_sse(
                    &mut socket,
                    &partial_sse_body("retry_completed_item", COMPLETED),
                )
                .await
            }
            2 => write_sse(&mut socket, &sse_body(&final_response())).await,
            _ => unreachable!(),
        }
    }
    requests
}

async fn serve_exhaustion(listener: TcpListener) -> Vec<Value> {
    let mut requests = Vec::new();
    for (round, text) in EXHAUSTED.into_iter().enumerate() {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .expect("bounded fixture accept")
            .unwrap();
        requests.push(read_json_request(&mut socket).await);
        write_sse(
            &mut socket,
            &partial_sse_body(&format!("retry_exhausted_{round}"), text),
        )
        .await;
    }
    requests
}

async fn configure(
    root: &Path,
    endpoint: &str,
    stream_max_retries: u64,
) -> proteus_core::core::AppConfig {
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("AGENTS.md"), "Use the configured tools.\n").unwrap();
    let mut config = fixture_config(endpoint).await;
    config
        .providers
        .get_mut(&config.active_provider)
        .unwrap()
        .stream = true;
    config
        .module_config
        .entry("workflow".to_owned())
        .or_default()
        .insert(
            "coding.codex_loop".to_owned(),
            json!({"stream_max_retries": stream_max_retries}),
        );
    config
        .module_config
        .get_mut("policy")
        .unwrap()
        .insert("codex_policy".to_owned(), json!({"allow": ["shell"]}));
    config.runtime.workflow_timeout_ms = 15_000;
    config
}

fn input_positions(request: &Value, predicate: impl Fn(&Value) -> bool) -> Vec<usize> {
    request["input"]
        .as_array()
        .expect("Responses input array")
        .iter()
        .enumerate()
        .filter_map(|(index, item)| predicate(item).then_some(index))
        .collect()
}

fn assert_retry_request(request: &Value) {
    for kind in ["function_call", "function_call_output"] {
        assert_eq!(
            input_positions(request, |item| {
                item["type"] == kind && item["call_id"] == CALL_ID
            })
            .len(),
            1,
            "retry contains one {kind} for the completed tool"
        );
    }
    let completed = input_positions(request, |item| {
        item["content"]
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| part["text"] == COMPLETED))
    });
    assert_eq!(completed.len(), 1);
    assert_eq!(request["input"][completed[0]]["phase"], "commentary");
    assert!(
        input_positions(request, |item| {
            item["content"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(|part| part["text"] == UNFINISHED))
        })
        .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_sse_disconnect_retries_same_turn_without_repeating_tool_effect() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = configure(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
        2,
    )
    .await;
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let server = tokio::spawn(serve_success(listener));
    let runtime = AgentRuntime::builder(config.clone(), root.path().join("workspace"))
        .with_config_path(Some(&config_path))
        .with_approval(Arc::new(ApprovingTransport))
        .build_async()
        .await
        .unwrap();

    let output = runtime.run(PROMPT.to_owned()).await.unwrap();
    assert_eq!(output.text, FINAL);
    let requests = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 3);
    assert_retry_request(&requests[2]);
    assert_eq!(
        std::fs::read_to_string(root.path().join("workspace").join(EFFECT_FILE)).unwrap(),
        EFFECT
    );

    let session_dir = runtime.session_dir().unwrap().to_owned();
    let projection = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(projection.history, runtime.history().await);
    assert_eq!(
        projection
            .records
            .iter()
            .filter(|record| matches!(
                &record.entry,
                JournalEntry::ModelResponseRecorded(response)
                    if matches!(response.outcome, ModelResponseOutcome::Error {
                        ref failure
                    } if failure.kind == ModelFailureKind::StreamDisconnected)
            ))
            .count(),
        1
    );
    let completed_messages: Vec<_> = projection
        .history
        .iter()
        .filter(|message| message.display_text() == COMPLETED)
        .collect();
    assert_eq!(completed_messages.len(), 1);
    assert_eq!(completed_messages[0].phase, Some(MessagePhase::Commentary));
    assert!(projection.history.iter().all(|message| {
        message.parts.iter().all(
        |part| !matches!(&part.payload, ContentPart::Text { text } if text.contains(UNFINISHED))
    )
    }));
    assert_eq!(
        projection
            .records
            .iter()
            .filter_map(|record| match &record.entry {
                JournalEntry::TurnSettled(settled) => Some(settled.status),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [TurnSettlementStatus::Success]
    );

    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        root.path().join("workspace"),
        Some(&config_path),
        session_dir.clone(),
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    assert_eq!(
        transcript
            .iter()
            .filter(|item| item.text == COMPLETED && item.phase == Some(MessagePhase::Commentary))
            .count(),
        1
    );
    assert!(
        transcript
            .iter()
            .all(|item| !item.text.contains(UNFINISHED))
    );

    std::fs::remove_file(root.path().join("workspace").join(EFFECT_FILE)).unwrap();
    let replay = replay_workflow(
        &session_dir,
        &config,
        &ModuleCatalog::from_config(&config).unwrap(),
        WorkflowReplayOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(replay.recorded.status, TurnSettlementStatus::Success);
    assert_eq!(replay.replay.status, TurnSettlementStatus::Success);
    assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
    assert!(replay.source_journal_unchanged);
    assert!(
        !root.path().join("workspace").join(EFFECT_FILE).exists(),
        "workflow replay must not execute the completed tool again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_sse_disconnect_stops_after_configured_retry_budget() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = configure(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
        2,
    )
    .await;
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let server = tokio::spawn(serve_exhaustion(listener));
    let runtime = AgentRuntime::builder(config.clone(), root.path().join("workspace"))
        .with_config_path(Some(&config_path))
        .with_approval(Arc::new(ApprovingTransport))
        .build_async()
        .await
        .unwrap();

    let error = runtime.run(PROMPT.to_owned()).await.unwrap_err();
    assert!(
        format!("{error:#}").contains("stream ended without a terminal event"),
        "{error:#}"
    );
    let requests = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 3, "initial request plus two retries");
    for (request_index, request) in requests.iter().enumerate() {
        for (message_index, text) in EXHAUSTED.iter().enumerate() {
            let count = input_positions(request, |item| {
                item["content"]
                    .as_array()
                    .is_some_and(|parts| parts.iter().any(|part| part["text"] == *text))
            })
            .len();
            assert_eq!(
                count,
                usize::from(message_index < request_index),
                "request {request_index} has the wrong completed failure progress"
            );
        }
        assert!(
            input_positions(request, |item| {
                item["content"]
                    .as_array()
                    .is_some_and(|parts| parts.iter().any(|part| part["text"] == UNFINISHED))
            })
            .is_empty()
        );
    }

    let session_dir = runtime.session_dir().unwrap().to_owned();
    let projection = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    for text in EXHAUSTED {
        assert_eq!(
            projection
                .history
                .iter()
                .filter(|message| message.display_text() == text)
                .count(),
            1
        );
    }
    assert!(projection.history.iter().all(|message| {
        message.parts.iter().all(
        |part| !matches!(&part.payload, ContentPart::Text { text } if text.contains(UNFINISHED))
    )
    }));
    assert_eq!(
        projection
            .records
            .iter()
            .filter(|record| matches!(
                &record.entry,
                JournalEntry::ModelResponseRecorded(response)
                    if matches!(response.outcome, ModelResponseOutcome::Error {
                        ref failure
                    } if failure.kind == ModelFailureKind::StreamDisconnected)
            ))
            .count(),
        3
    );
    assert_eq!(
        projection
            .records
            .iter()
            .filter_map(|record| match &record.entry {
                JournalEntry::TurnSettled(settled) => Some(settled.status),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [TurnSettlementStatus::Error]
    );

    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        root.path().join("workspace"),
        Some(&config_path),
        session_dir.clone(),
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    for text in EXHAUSTED {
        assert_eq!(
            transcript.iter().filter(|item| item.text == text).count(),
            1
        );
    }
    assert!(
        transcript
            .iter()
            .all(|item| !item.text.contains(UNFINISHED))
    );

    let replay = replay_workflow(
        &session_dir,
        &config,
        &ModuleCatalog::from_config(&config).unwrap(),
        WorkflowReplayOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(replay.recorded.status, TurnSettlementStatus::Error);
    assert_eq!(replay.replay.status, TurnSettlementStatus::Error);
    assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
    assert!(replay.source_journal_unchanged);
}
