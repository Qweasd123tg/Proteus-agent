//! Completed assistant items survive an accepted SSE response which ends before
//! `response.completed`, including a real process restart before continuation.
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use proteus_contracts::model_standard::{ContentPart, MessagePhase, MessageRole, ModelFailureKind};
use proteus_core::core::{
    AgentRuntime, AppConfig, JournalEntry, ModuleCatalog, SessionStore, ToolCallRecordPhase,
    TurnSettlementStatus, WorkflowReplayOptions, replay_workflow,
};
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, net::TcpListener, process::Command};

use super::{
    CapturedEvents,
    direct_tool_surface::{ApprovingTransport, fixture_config},
    read_json_request, sse_body,
};

const CHILD_ROOT: &str = "PROTEUS_CODEX_PARTIAL_SSE_TEST_ROOT";
const INITIAL_PROMPT: &str = "Покажи ход работы и затем продолжи.";
const CONTINUE_PROMPT: &str = "Продолжай после обрыва.";
const COMMENTARY: &str = "Первый завершённый фрагмент.";
const EARLY_FINAL: &str = "Завершённый ответ до обрыва.";
const UNFINISHED: &str = "этот незавершённый текст нельзя сохранять";
const CALL_ID: &str = "call_partial_sse_once";
const TARGET_FILE: &str = "partial-sse-effect.txt";
const EFFECT: &str = "executed once after continuation\n";
const FINAL: &str = "Продолжение завершено.";

fn completed_message(id: &str, phase: &str, text: &str) -> Value {
    json!({
        "type": "message", "id": id, "status": "completed", "role": "assistant",
        "phase": phase,
        "content": [{"type": "output_text", "text": text, "annotations": []}],
    })
}

fn partial_sse_body() -> String {
    let commentary = completed_message("partial_commentary", "commentary", COMMENTARY);
    let early_final = completed_message("partial_final", "final_answer", EARLY_FINAL);
    let unfinished = json!({
        "type": "message", "id": "partial_unfinished", "status": "in_progress",
        "role": "assistant", "phase": "commentary", "content": [],
    });
    [
        format!(
            "event: response.output_item.done\ndata: {}\n\n",
            json!({"output_index": 0, "item": commentary})
        ),
        format!(
            "event: response.output_item.done\ndata: {}\n\n",
            json!({"output_index": 1, "item": early_final})
        ),
        format!(
            "event: response.output_item.added\ndata: {}\n\n",
            json!({"output_index": 2, "item": unfinished})
        ),
        format!(
            "event: response.output_text.delta\ndata: {}\n\n",
            json!({
                "output_index": 2, "item_id": "partial_unfinished", "content_index": 0,
                "delta": UNFINISHED,
            })
        ),
    ]
    .concat()
}

fn tool_response() -> Value {
    json!({
        "id": "partial_sse_tool_response", "object": "response", "status": "completed",
        "output": [{
            "type": "function_call", "id": "partial_sse_tool", "status": "completed",
            "call_id": CALL_ID, "name": "write_file",
            "arguments": serde_json::to_string(&json!({"path": TARGET_FILE, "content": EFFECT})).unwrap(),
        }],
    })
}

fn final_response() -> Value {
    json!({
        "id": "partial_sse_success", "object": "response", "status": "completed",
        "output": [completed_message("partial_sse_success_message", "final_answer", FINAL)],
    })
}

async fn serve(listener: TcpListener) -> Vec<Value> {
    let mut requests = Vec::new();
    for round in 0..3 {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(15), listener.accept())
            .await
            .expect("bounded fixture accept")
            .unwrap();
        requests.push(read_json_request(&mut socket).await);
        let body = match round {
            0 => partial_sse_body(),
            1 => sse_body(&tool_response()),
            2 => sse_body(&final_response()),
            _ => unreachable!(),
        };
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
    requests
}

async fn configure(root: &Path, endpoint: &str) -> AppConfig {
    std::fs::create_dir(root.join("workspace")).unwrap();
    std::fs::write(
        root.join("workspace/AGENTS.md"),
        "Use the configured tools.\n",
    )
    .unwrap();
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
            json!({"stream_max_retries": 0}),
        );
    std::fs::write(
        root.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    config
}

fn completed_assistant_messages(
    projection: &proteus_core::core::JournalProjection,
) -> Vec<&proteus_contracts::model_standard::CanonicalMessage> {
    projection
        .history
        .iter()
        .filter(|message| {
            message.role == MessageRole::Assistant
                && matches!(message.display_text().as_str(), COMMENTARY | EARLY_FINAL)
        })
        .collect()
}

fn assert_failed_history(projection: &proteus_core::core::JournalProjection) {
    let messages = completed_assistant_messages(projection);
    assert_eq!(messages.len(), 2, "both completed SSE items are durable");
    assert_eq!(messages[0].display_text(), COMMENTARY);
    assert_eq!(messages[0].phase, Some(MessagePhase::Commentary));
    assert_eq!(messages[1].display_text(), EARLY_FINAL);
    assert_eq!(messages[1].phase, Some(MessagePhase::FinalAnswer));
    assert!(projection.history.iter().all(|message| {
        !message.parts.iter().any(
            |part| matches!(&part.payload, ContentPart::Text { text } if text.contains(UNFINISHED)),
        )
    }));
    let settlements: Vec<_> = projection
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::TurnSettled(settled) => Some(settled.status),
            _ => None,
        })
        .collect();
    assert_eq!(
        settlements.first(),
        Some(&TurnSettlementStatus::Error),
        "a completed final-answer item cannot synthesize turn success"
    );
    assert_eq!(
        projection
            .records
            .iter()
            .filter(|record| matches!(
                &record.entry,
                JournalEntry::ModelResponseRecorded(response)
                    if matches!(&response.outcome,
                        proteus_core::core::ModelResponseOutcome::Error { failure }
                            if failure.kind == ModelFailureKind::StreamDisconnected)
            ))
            .count(),
        1,
        "the intentionally disabled retry path retains the typed stream failure"
    );
}

fn assert_live_completions(
    projection: &proteus_core::core::JournalProjection,
    captured: &CapturedEvents,
) {
    let events = captured.0.lock().unwrap();
    for message in completed_assistant_messages(projection) {
        let completions: Vec<_> = events
            .iter()
            .filter_map(|envelope| match &envelope.event {
                proteus_contracts::domain::Event::AssistantMessageCompleted {
                    message_id,
                    phase,
                    text,
                } if *message_id == message.id => Some((*phase, text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            completions,
            [(message.phase, message.display_text())],
            "live completion uses the durable message id and phase"
        );
    }
}

fn assert_resume_request(request: &Value) {
    let input = request["input"].as_array().expect("Responses input array");
    let text_positions = |needle: &str| {
        input
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                item["content"]
                    .as_array()
                    .is_some_and(|parts| parts.iter().any(|part| part["text"] == needle))
                    .then_some(index)
            })
            .collect::<Vec<_>>()
    };
    let commentary = text_positions(COMMENTARY);
    let early_final = text_positions(EARLY_FINAL);
    let continuation = text_positions(CONTINUE_PROMPT);
    assert_eq!(commentary.len(), 1);
    assert_eq!(early_final.len(), 1);
    assert_eq!(continuation.len(), 1);
    assert!(commentary[0] < early_final[0] && early_final[0] < continuation[0]);
    assert!(text_positions(UNFINISHED).is_empty());
    assert_eq!(input[commentary[0]]["phase"], "commentary");
    assert_eq!(input[early_final[0]]["phase"], "final_answer");
}

async fn assert_final_state(config: &AppConfig, root: &Path, session_dir: &Path) {
    let projection = SessionStore::open(session_dir.to_owned())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_failed_history(&projection);
    let saved = completed_assistant_messages(&projection);
    let saved_ids: Vec<_> = saved.iter().map(|message| message.id).collect();
    let settlements: Vec<_> = projection
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::TurnSettled(settled) => Some((record.turn_id.unwrap(), settled.status)),
            _ => None,
        })
        .collect();
    assert_eq!(
        settlements
            .iter()
            .map(|(_, status)| *status)
            .collect::<Vec<_>>(),
        [TurnSettlementStatus::Error, TurnSettlementStatus::Success]
    );
    assert_eq!(
        projection
            .records
            .iter()
            .filter(|record| matches!(
                &record.entry,
                JournalEntry::ToolCallRecorded(tool)
                    if tool.phase == ToolCallRecordPhase::Requested && tool.call.id == CALL_ID
            ))
            .count(),
        1
    );
    assert_eq!(
        projection
            .records
            .iter()
            .filter(|record| matches!(
                &record.entry,
                JournalEntry::ToolResultRecorded(result) if result.result.call_id == CALL_ID
            ))
            .count(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(root.join("workspace").join(TARGET_FILE)).unwrap(),
        EFFECT
    );

    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        root.join("workspace"),
        Some(&root.join("config.json")),
        session_dir.to_owned(),
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    for (id, text, phase) in [
        (saved_ids[0], COMMENTARY, MessagePhase::Commentary),
        (saved_ids[1], EARLY_FINAL, MessagePhase::FinalAnswer),
    ] {
        let item = transcript
            .iter()
            .find(|item| item.message_id == Some(id))
            .expect("cold transcript preserves the original message id");
        assert_eq!(item.text, text);
        assert_eq!(item.phase, Some(phase));
        assert!(!item.streaming);
    }
    assert!(
        transcript
            .iter()
            .all(|item| !item.text.contains(UNFINISHED))
    );

    std::fs::remove_file(root.join("workspace").join(TARGET_FILE)).unwrap();

    for (turn_id, status) in settlements {
        let replay = replay_workflow(
            session_dir,
            config,
            &ModuleCatalog::from_config(config).unwrap(),
            WorkflowReplayOptions {
                turn_id: Some(turn_id),
            },
        )
        .await
        .unwrap();
        assert_eq!(replay.recorded.status, status);
        assert_eq!(replay.replay.status, status);
        assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
        assert!(replay.source_journal_unchanged);
    }
    assert!(
        !root.join("workspace").join(TARGET_FILE).exists(),
        "workflow replay must not execute the tool a second time"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_messages_survive_partial_sse_and_warm_continuation() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = configure(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
    )
    .await;
    let server = tokio::spawn(serve(listener));
    let config_path = root.path().join("config.json");
    let captured = Arc::new(CapturedEvents::default());
    let runtime = AgentRuntime::builder(config.clone(), root.path().join("workspace"))
        .with_config_path(Some(&config_path))
        .with_approval(Arc::new(ApprovingTransport))
        .with_event_sink(captured.clone())
        .build_async()
        .await
        .unwrap();

    let error = runtime.run(INITIAL_PROMPT.to_owned()).await.unwrap_err();
    assert!(format!("{error:#}").contains("stream ended without a terminal event"));
    let session_dir = runtime.session_dir().unwrap().to_owned();
    let failed = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_failed_history(&failed);
    assert_live_completions(&failed, &captured);
    let failed_ids: Vec<_> = completed_assistant_messages(&failed)
        .iter()
        .map(|message| message.id)
        .collect();
    runtime.run(CONTINUE_PROMPT.to_owned()).await.unwrap();
    let requests = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 3, "accepted partial SSE is not retried");
    assert_resume_request(&requests[1]);
    let restored = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(
        completed_assistant_messages(&restored)
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>(),
        failed_ids,
        "warm continuation preserves message ids"
    );
    assert_final_state(&config, root.path(), &session_dir).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partial_sse_cold_runtime_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT).map(PathBuf::from) else {
        return;
    };
    let config_path = root.join("config.json");
    let config: AppConfig = serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    let note_path = root.join("resume.json");
    let mut builder =
        AgentRuntime::builder(config, root.join("workspace")).with_config_path(Some(&config_path));
    let first = !note_path.exists();
    let thread_id = if first {
        proteus_contracts::domain::new_thread_id()
    } else {
        let note: Value = serde_json::from_slice(&std::fs::read(&note_path).unwrap()).unwrap();
        let thread_id = serde_json::from_value(note["thread_id"].clone()).unwrap();
        builder = builder
            .resume_from_session_dir(
                PathBuf::from(note["session_dir"].as_str().unwrap()),
                thread_id,
            )
            .unwrap();
        thread_id
    };
    if first {
        builder = builder.with_session_ids(proteus_contracts::domain::new_session_id(), thread_id);
    }
    let runtime = builder
        .with_approval(Arc::new(ApprovingTransport))
        .build_async()
        .await
        .unwrap();
    let result = runtime
        .run(
            if first {
                INITIAL_PROMPT
            } else {
                CONTINUE_PROMPT
            }
            .to_owned(),
        )
        .await;
    if first {
        let error = result.unwrap_err();
        assert!(format!("{error:#}").contains("stream ended without a terminal event"));
        std::fs::write(
            note_path,
            json!({
                "session_dir": runtime.session_dir().unwrap(),
                "thread_id": thread_id,
            })
            .to_string(),
        )
        .unwrap();
    } else {
        assert_eq!(result.unwrap().text, FINAL);
    }
}

async fn run_child(root: &Path) {
    let output = tokio::time::timeout(
        Duration::from_secs(25),
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "partial_sse_recovery::partial_sse_cold_runtime_child",
                "--nocapture",
            ])
            .env(CHILD_ROOT, root)
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("bounded child runtime")
    .unwrap();
    assert!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_messages_survive_partial_sse_and_cold_process_restart() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = configure(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
    )
    .await;
    let server = tokio::spawn(serve(listener));
    run_child(root.path()).await;
    let note: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("resume.json")).unwrap()).unwrap();
    let session_dir = PathBuf::from(note["session_dir"].as_str().unwrap());
    let failed = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_failed_history(&failed);
    let failed_ids: Vec<_> = completed_assistant_messages(&failed)
        .iter()
        .map(|message| message.id)
        .collect();
    run_child(root.path()).await;
    let requests = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 3, "accepted partial SSE is not retried");
    assert_resume_request(&requests[1]);
    let restored = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(
        completed_assistant_messages(&restored)
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>(),
        failed_ids,
        "cold continuation preserves message ids"
    );
    assert_final_state(&config, root.path(), &session_dir).await;
}
