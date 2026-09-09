//! Retry a model HTTP request through real workers without replaying the tool.
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use proteus_contracts::{
    contracts::CancellationToken,
    model_standard::{ContentPart, ModelFailureKind},
};
use proteus_core::core::{
    AgentRuntime, JournalEntry, ModelResponseOutcome, ModuleCatalog, SessionStore,
    TurnSettlementStatus, WorkflowReplayOptions, replay_workflow,
};
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, net::TcpListener, sync::Notify};

use super::direct_tool_surface::{ApprovingTransport, fixture_config};

const CALL: &str = "call_once_before_retry";
const PROMPT: &str = "Выполни изменение и проверь результат.";
const FINAL: &str = "Запрос восстановлен без повторного изменения.";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscription_quota_stops_without_retry_and_preserves_error_replay() {
    check(Mode::SubscriptionQuota).await;
}

#[derive(Clone, Copy)]
enum Mode {
    Json,
    Sse,
    Cancel,
    ModelTimeout,
    PartialSse,
    SubscriptionQuota,
}

impl Mode {
    fn streaming(self) -> bool {
        matches!(self, Self::Sse | Self::PartialSse | Self::SubscriptionQuota)
    }
    fn interrupted(self) -> bool {
        matches!(self, Self::Cancel | Self::ModelTimeout)
    }
    fn status(self) -> TurnSettlementStatus {
        match self {
            Self::Json | Self::Sse => TurnSettlementStatus::Success,
            Self::Cancel => TurnSettlementStatus::Canceled,
            Self::ModelTimeout | Self::PartialSse | Self::SubscriptionQuota => {
                TurnSettlementStatus::Error
            }
        }
    }
}

fn response(first: bool) -> Value {
    json!({"status": "completed", "output": if first {
        json!([{"type": "function_call", "call_id": CALL, "name": "shell",
            "arguments": serde_json::to_string(&json!({"command": "printf x >> effects.log; printf 'committed\\n'"})).unwrap()}])
    } else {
        json!([{"id": "retry_final", "type": "message", "role": "assistant", "phase": "final_answer",
            "content": [{"type": "output_text", "text": FINAL}]}])
    }})
}

async fn serve(
    listener: TcpListener,
    mode: Mode,
    requests: Arc<Mutex<Vec<Value>>>,
    failed: Arc<Notify>,
) {
    for index in 0.. {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = super::read_json_request(&mut socket).await;
        requests.lock().unwrap().push(request);
        let (status, content_type, body) = if index >= 1 && matches!(mode, Mode::SubscriptionQuota)
        {
            (
                "429 Too Many Requests",
                "application/json",
                json!({"error": {
                    "type": "usage_limit_reached", "message": "subscription allowance exhausted"
                }})
                .to_string(),
            )
        } else if index == 1 || (index > 1 && matches!(mode, Mode::ModelTimeout)) {
            (
                "500 Internal Server Error",
                "application/json",
                json!({"error": {"message": "temporary failure"}}).to_string(),
            )
        } else if index > 1 && matches!(mode, Mode::PartialSse) {
            // A completed item is followed by EOF without response.completed.
            // HTTP retries must never resubmit this accepted response.
            (
                "200 OK",
                "text/event-stream",
                format!(
                    "event: response.output_item.done\ndata: {}\n\n",
                    json!({"output_index": 0, "item": response(false)["output"][0]})
                ),
            )
        } else {
            let response = response(index == 0);
            if mode.streaming() {
                ("200 OK", "text/event-stream", super::sse_body(&response))
            } else {
                ("200 OK", "application/json", response.to_string())
            }
        };
        socket.write_all(format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
        ).as_bytes()).await.unwrap();
        if index == 1 {
            failed.notify_one();
        }
    }
}

async fn check(mode: Mode) {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = fixture_config(&format!("http://{}", listener.local_addr().unwrap())).await;
    if matches!(mode, Mode::SubscriptionQuota) {
        super::use_subscription(&mut config, root.path());
    }
    config
        .providers
        .get_mut(&config.active_provider)
        .unwrap()
        .stream = mode.streaming();
    if matches!(mode, Mode::PartialSse) {
        // This scenario isolates the provider HTTP retry boundary. The workflow
        // retry path has its own process regression in `stream_recovery`.
        config
            .module_config
            .entry("workflow".to_owned())
            .or_default()
            .insert(
                "coding.codex_loop".to_owned(),
                json!({"stream_max_retries": 0}),
            );
    }
    config
        .module_config
        .get_mut("policy")
        .unwrap()
        .insert("codex_policy".to_owned(), json!({"allow": ["shell"]}));
    if matches!(mode, Mode::ModelTimeout) {
        config.runtime.model_timeout_ms = 1_000;
    }
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let runtime = Arc::new(
        AgentRuntime::builder(config.clone(), workspace.clone())
            .with_config_path(Some(&config_path))
            .with_approval(Arc::new(ApprovingTransport))
            .build_async()
            .await
            .unwrap(),
    );
    let requests = Arc::new(Mutex::new(Vec::new()));
    let failed = Arc::new(Notify::new());
    let server = tokio::spawn(serve(listener, mode, requests.clone(), failed.clone()));
    let cancel = CancellationToken::new();
    let run = tokio::spawn({
        let runtime = runtime.clone();
        let cancel = cancel.clone();
        async move {
            runtime
                .run_with_cancellation(PROMPT.to_owned(), cancel)
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(10), failed.notified())
        .await
        .unwrap();
    if matches!(mode, Mode::Cancel) {
        cancel.cancel();
    }
    let result = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .unwrap()
        .unwrap();
    if mode.status() == TurnSettlementStatus::Success {
        assert_eq!(result.unwrap().text, FINAL);
    } else {
        let error = format!("{:#}", result.unwrap_err());
        if matches!(mode, Mode::SubscriptionQuota) {
            assert!(
                error.contains("subscription allowance exhausted"),
                "{error}"
            );
        }
        if matches!(mode, Mode::PartialSse) {
            assert!(
                error.contains("stream ended without a terminal event"),
                "{error}"
            );
        }
        if matches!(mode, Mode::ModelTimeout) {
            assert!(
                error.contains("model request timed out after 1000ms"),
                "{error}"
            );
        }
    }
    let at_settlement = requests.lock().unwrap().len();
    if mode.interrupted() || matches!(mode, Mode::PartialSse) {
        // Longer than the largest default backoff (1760 ms): no worker
        // request may outlive the operation which owned its retries.
        tokio::time::sleep(Duration::from_millis(1_800)).await;
        assert_eq!(requests.lock().unwrap().len(), at_settlement);
    }
    server.abort();
    let requests = requests.lock().unwrap().clone();
    match mode {
        Mode::Cancel | Mode::SubscriptionQuota => assert_eq!(requests.len(), 2),
        Mode::ModelTimeout => assert!(
            (3..=4).contains(&requests.len()),
            "{} requests",
            requests.len()
        ),
        _ => assert_eq!(requests.len(), 3),
    }
    assert!(
        requests[1..].windows(2).all(|pair| pair[0] == pair[1]),
        "HTTP retries changed the request"
    );
    let input = requests[1]["input"].as_array().unwrap();
    for kind in ["function_call", "function_call_output"] {
        assert_eq!(
            input
                .iter()
                .filter(|item| item["type"] == kind && item["call_id"] == CALL)
                .count(),
            1
        );
    }
    assert_eq!(
        std::fs::read_to_string(workspace.join("effects.log")).unwrap(),
        "x"
    );
    let session_dir = runtime.session_dir().unwrap().to_owned();
    let projection = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(projection.history, runtime.history().await);
    assert!(projection.unresolved_tool_calls.is_empty());
    if !matches!(mode, Mode::Cancel) {
        assert!(projection.interrupted_model_exchanges.is_empty());
    }
    if matches!(mode, Mode::ModelTimeout) {
        let errors: Vec<_> = projection
            .records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| match &record.entry {
                JournalEntry::ModelResponseRecorded(response) => match &response.outcome {
                    ModelResponseOutcome::Error { failure } => {
                        Some((index, response.exchange_id, &failure.message))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 1, "deadline records one terminal model error");
        let (error_index, exchange_id, message) = errors[0];
        assert_eq!(message, "model request timed out after 1000ms");
        let request_index = projection.records.iter().position(|record| matches!(
            &record.entry, JournalEntry::ModelRequestRecorded(request) if request.exchange_id == exchange_id
        )).expect("deadline outcome belongs to its recorded request");
        let settlement_index = projection
            .records
            .iter()
            .position(|record| matches!(&record.entry, JournalEntry::TurnSettled(_)))
            .expect("settlement");
        assert!(request_index < error_index && error_index < settlement_index);
    }
    if matches!(mode, Mode::PartialSse) {
        assert_eq!(
            projection
                .records
                .iter()
                .filter(|record| matches!(
                    &record.entry,
                    JournalEntry::ModelResponseRecorded(response)
                        if matches!(&response.outcome, ModelResponseOutcome::Error { failure }
                            if failure.kind == ModelFailureKind::StreamDisconnected)
                ))
                .count(),
            1
        );
    }
    assert_eq!(
        projection
            .records
            .iter()
            .filter(|record| matches!(&record.entry,
        JournalEntry::ToolResultRecorded(tool) if tool.result.call_id == CALL && tool.result.ok))
            .count(),
        1
    );
    assert_eq!(
        projection
            .records
            .iter()
            .filter(|record| matches!(&record.entry, JournalEntry::ModelRequestRecorded(_)))
            .count(),
        2,
        "HTTP attempts belong to one model exchange"
    );
    let settlements: Vec<_> = projection
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::TurnSettled(settled) => Some(settled.status),
            _ => None,
        })
        .collect();
    assert_eq!(settlements, [mode.status()]);
    assert_eq!(
        projection
            .history
            .iter()
            .flat_map(|message| &message.parts)
            .filter(|part| matches!(&part.payload,
        ContentPart::ToolResult { result } if result.call_id == CALL && result.ok))
            .count(),
        1
    );
    let app = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        workspace.clone(),
        Some(&config_path),
        session_dir.clone(),
    )
    .await
    .unwrap();
    let transcript = app.transcript().await.unwrap();
    assert_eq!(
        transcript
            .iter()
            .filter_map(|message| message.tool.as_ref())
            .filter(|tool| tool.call_id == CALL && tool.status == "done")
            .count(),
        1
    );
    if !matches!(mode, Mode::Cancel) {
        let replay = replay_workflow(
            &session_dir,
            &config,
            &ModuleCatalog::from_config(&config).unwrap(),
            WorkflowReplayOptions::default(),
        )
        .await
        .unwrap();
        assert!(replay.comparison.matched, "{replay:#?}");
        assert!(replay.source_journal_unchanged);
        assert_eq!(
            std::fs::read_to_string(workspace.join("effects.log")).unwrap(),
            "x",
            "workflow replay must not repeat the tool effect"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_500_then_json_success_does_not_repeat_the_tool() {
    check(Mode::Json).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_500_then_sse_success_does_not_repeat_the_tool() {
    check(Mode::Sse).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_stops_process_http_retries_and_preserves_the_tool() {
    check(Mode::Cancel).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_deadline_bounds_all_process_http_attempts() {
    check(Mode::ModelTimeout).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_partial_sse_is_not_resubmitted_by_http_retry() {
    check(Mode::PartialSse).await;
}
