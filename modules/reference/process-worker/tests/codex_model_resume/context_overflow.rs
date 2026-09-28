//! An ordinary model overflow settles the turn; the next turn compacts first.
//! All model traffic goes to the loopback Responses fixture.
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use proteus_contracts::{
    contracts::ModelCallOrigin,
    domain::{new_session_id, new_thread_id},
    model_standard::{ContentPart, MessageRole, ModelFailureKind},
};
use proteus_core::core::{
    AgentRuntime, AppConfig, JournalEntry, ModelResponseOutcome, ModuleCatalog, SessionStore,
    ToolCallRecordPhase, TurnSettlementStatus, WorkflowReplayOptions, read_eval_report,
    replay_workflow,
};
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, net::TcpListener, process::Command};

use super::direct_tool_surface::{ApprovingTransport, fixture_config};

const CHILD_ROOT: &str = "PROTEUS_CODEX_CONTEXT_OVERFLOW_ROOT";
const TARGET_FILE: &str = "written-before-overflow.txt";
const WRITTEN_CONTENT: &str = "one write before context overflow\n";
const FIRST_TASK: &str = "Запиши контрольный файл.";
const NEXT_TASK: &str = "Продолжай после переполнения контекста.";
const SUMMARY_TEXT: &str = "Контрольный файл уже записан; продолжи без повтора.";
const FINAL_TEXT: &str = "Продолжение завершено.";
const COMPACTION_PROMPT: &str =
    include_str!("../../../codex-compactor/src/upstream/compact_prompt.md");
const TRIGGER_TOKENS: usize = 90_000;

fn tool_response() -> Value {
    json!({
        "id": "overflow_tool",
        "object": "response",
        "status": "completed",
        "output": [{
            "type": "function_call",
            "id": "overflow_write_item",
            "status": "completed",
            "call_id": "call_write_before_overflow",
            "name": "write_file",
            "arguments": serde_json::to_string(&json!({
                "path": TARGET_FILE,
                "content": WRITTEN_CONTENT,
            })).unwrap(),
        }],
        "usage": {"input_tokens": 100, "output_tokens": 10}
    })
}

fn message_response(id: &str, text: &str, input_tokens: u32, output_tokens: u32) -> Value {
    json!({
        "id": id,
        "object": "response",
        "status": "completed",
        "output": [{
            "type": "message",
            "id": format!("{id}_message"),
            "status": "completed",
            "role": "assistant",
            "phase": "final_answer",
            "content": [{"type": "output_text", "text": text, "annotations": []}],
        }],
        "usage": {"input_tokens": input_tokens, "output_tokens": output_tokens}
    })
}

async fn serve(listener: TcpListener) -> Vec<Value> {
    let replies = [
        (200, tool_response()),
        (
            400,
            json!({"error": {
                "code": "context_length_exceeded",
                "message": "fixture ordinary request overflow"
            }}),
        ),
        (
            200,
            message_response("overflow_summary", SUMMARY_TEXT, 13, 4),
        ),
        (200, message_response("overflow_final", FINAL_TEXT, 17, 5)),
    ];
    let mut requests = Vec::new();
    for (status, body) in replies {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .expect("bounded fixture accept")
            .expect("fixture socket");
        requests.push(super::read_json_request(&mut socket).await);
        let body = body.to_string();
        let status_text = if status == 200 { "OK" } else { "Fixture Error" };
        socket
            .write_all(
                format!(
                    "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .expect("fixture reply");
    }
    requests
}

async fn configure(root: &Path, endpoint: &str) -> AppConfig {
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).expect("workspace");
    std::fs::write(workspace.join("AGENTS.md"), "Use configured tools.\n").expect("agents");
    let mut config = fixture_config(endpoint).await;
    let model = config
        .module_config
        .get_mut("model")
        .and_then(|models| models.get_mut("openai"))
        .expect("OpenAI config");
    model["request_max_retries"] = json!(0);
    model["max_input_tokens"] = json!(100_000);
    config
        .module_config
        .get_mut("compactor")
        .and_then(|modules| modules.get_mut("codex"))
        .expect("Codex compactor config")["trigger_tokens"] = json!(TRIGGER_TOKENS);
    std::fs::write(
        root.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .expect("config");
    config
}

fn request_items(request: &Value) -> &[Value] {
    request["input"].as_array().expect("Responses input")
}

fn contains_text(items: &[Value], text: &str) -> bool {
    items.iter().any(|item| {
        item["content"].as_array().is_some_and(|parts| {
            parts.iter().any(|part| {
                part["text"]
                    .as_str()
                    .is_some_and(|value| value.contains(text))
            })
        })
    })
}

fn assert_requests(requests: &[Value]) {
    assert_eq!(requests.len(), 4, "one overflow without a retry");
    let failed_input = request_items(&requests[1]);
    let calls = failed_input
        .iter()
        .filter(|item| {
            item["type"] == "function_call" && item["call_id"] == "call_write_before_overflow"
        })
        .count();
    let results = failed_input
        .iter()
        .filter(|item| {
            item["type"] == "function_call_output"
                && item["call_id"] == "call_write_before_overflow"
        })
        .count();
    assert_eq!((calls, results), (1, 1));

    // The current heuristic is comfortably below the configured threshold.
    // A summary here must be driven by the previous typed overflow.
    assert!(
        serde_json::to_string(failed_input).unwrap().len() / 4 < TRIGGER_TOKENS,
        "fixture must not trigger ordinary estimate-based compaction"
    );
    let summary = &requests[2];
    assert_eq!(summary["tool_choice"], "none");
    assert!(summary.get("tools").is_none());
    let summary_items = request_items(summary);
    assert!(
        !contains_text(summary_items, NEXT_TASK),
        "incoming user is excluded from pre-turn summary"
    );
    assert!(summary_items.iter().any(|item| {
        item["type"] == "function_call_output" && item["call_id"] == "call_write_before_overflow"
    }));
    assert_eq!(
        summary_items.last().expect("summary prompt")["content"][0]["text"],
        COMPACTION_PROMPT
    );

    let resumed = &requests[3];
    assert_eq!(resumed["tool_choice"], "auto");
    assert!(
        resumed["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty())
    );
    let resumed_items = request_items(resumed);
    assert!(contains_text(resumed_items, NEXT_TASK));
    assert_eq!(
        resumed_items.last().unwrap()["content"][0]["text"],
        NEXT_TASK
    );
    assert!(contains_text(resumed_items, SUMMARY_TEXT));
    assert!(
        resumed_items
            .iter()
            .all(|item| { item["call_id"] != "call_write_before_overflow" })
    );
}

async fn assert_evidence(config: &AppConfig, root: &Path, session_dir: &Path, requests: &[Value]) {
    assert_requests(requests);
    assert_eq!(
        std::fs::read_to_string(root.join("workspace").join(TARGET_FILE)).unwrap(),
        WRITTEN_CONTENT
    );
    let projection = SessionStore::open(session_dir.to_owned())
        .expect("cold session")
        .load_projection()
        .expect("cold projection");
    let origins: Vec<_> = projection
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::ModelRequestRecorded(request) => Some(request.origin),
            _ => None,
        })
        .collect();
    assert_eq!(
        origins,
        [
            ModelCallOrigin::Direct,
            ModelCallOrigin::Direct,
            ModelCallOrigin::Compactor,
            ModelCallOrigin::Direct
        ]
    );
    assert_eq!(
        projection
            .records
            .iter()
            .filter(|record| matches!(
                &record.entry,
                JournalEntry::ModelResponseRecorded(response)
                    if matches!(&response.outcome, ModelResponseOutcome::Error { failure }
                        if failure.kind == ModelFailureKind::ContextWindowExceeded)
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
                JournalEntry::ToolCallRecorded(call)
                    if call.phase == ToolCallRecordPhase::Requested
                        && call.call.id == "call_write_before_overflow"
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
                JournalEntry::ToolResultRecorded(result)
                    if result.result.call_id == "call_write_before_overflow"
            ))
            .count(),
        1
    );
    assert!(projection.unresolved_tool_calls.is_empty());
    assert!(projection.records.iter().any(|record| matches!(
        &record.entry,
        JournalEntry::HistoryMutated(mutation)
            if mutation.compaction.as_ref().is_some_and(|report| report.changed)
    )));
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
    assert!(projection.history.iter().any(|message| {
        message.role == MessageRole::User && message.display_text() == NEXT_TASK
    }));
    assert!(
        projection
            .history
            .iter()
            .any(|message| { message.display_text().contains(SUMMARY_TEXT) })
    );
    assert!(
        projection
            .history
            .iter()
            .any(|message| { message.display_text() == FINAL_TEXT })
    );
    assert!(
        projection
            .history
            .iter()
            .all(|message| message.parts.iter().all(|part| {
                !matches!(
                    part.payload,
                    ContentPart::ToolCall { .. } | ContentPart::ToolResult { .. }
                )
            }))
    );

    let eval = read_eval_report(session_dir).expect("eval report");
    assert_eq!(eval.model_calls, 4);
    assert_eq!(eval.provider_input_tokens, 130);
    assert_eq!(eval.provider_output_tokens, 19);
    let catalog = ModuleCatalog::from_config(config).expect("catalog");
    for (turn_id, status) in settlements {
        let replay = replay_workflow(
            session_dir,
            config,
            &catalog,
            WorkflowReplayOptions {
                turn_id: Some(turn_id),
            },
        )
        .await
        .expect("workflow replay");
        assert_eq!(replay.recorded.status, status);
        assert_eq!(replay.replay.status, status);
        assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
        assert!(replay.source_journal_unchanged);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_overflow_compacts_on_warm_next_turn() {
    for stored in [true, false] {
        let root = tempfile::tempdir().expect("test root");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let config = configure(
            root.path(),
            &format!("http://{}", listener.local_addr().unwrap()),
        )
        .await;
        let server = tokio::spawn(serve(listener));
        let config_path = root.path().join("config.json");
        let builder = AgentRuntime::builder(config.clone(), root.path().join("workspace"))
            .with_approval(Arc::new(ApprovingTransport));
        let builder = if stored {
            builder.with_config_path(Some(&config_path))
        } else {
            builder
        };
        let runtime = builder.build_async().await.expect("runtime");
        let error = runtime.run(FIRST_TASK.to_owned()).await.unwrap_err();
        assert!(format!("{error:#}").contains("fixture ordinary request overflow"));
        let output = runtime.run(NEXT_TASK.to_owned()).await.expect("next turn");
        assert_eq!(output.text, FINAL_TEXT);
        let requests = server.await.expect("fixture server");
        if stored {
            let session_dir = runtime.session_dir().expect("session dir");
            assert_evidence(&config, root.path(), &session_dir, &requests).await;
        } else {
            assert!(runtime.session_dir().is_none());
            assert_requests(&requests);
            assert_eq!(
                std::fs::read_to_string(root.path().join("workspace").join(TARGET_FILE)).unwrap(),
                WRITTEN_CONTENT
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_overflow_cold_runtime_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT).map(PathBuf::from) else {
        return;
    };
    let config_path = root.join("config.json");
    let config: AppConfig =
        serde_json::from_slice(&std::fs::read(&config_path).expect("config")).unwrap();
    let note_path = root.join("resume.json");
    let first = !note_path.exists();
    let mut builder =
        AgentRuntime::builder(config, root.join("workspace")).with_config_path(Some(&config_path));
    if first {
        let thread_id = new_thread_id();
        builder = builder.with_session_ids(new_session_id(), thread_id);
        std::fs::write(
            root.join("thread.json"),
            serde_json::to_vec(&thread_id).unwrap(),
        )
        .unwrap();
    } else {
        let note: Value = serde_json::from_slice(&std::fs::read(&note_path).unwrap()).unwrap();
        builder = builder
            .resume_from_session_dir(
                PathBuf::from(note["session_dir"].as_str().unwrap()),
                serde_json::from_value(note["thread_id"].clone()).unwrap(),
            )
            .expect("resume");
    }
    let runtime = builder
        .with_approval(Arc::new(ApprovingTransport))
        .build_async()
        .await
        .expect("child runtime");
    if first {
        let error = runtime.run(FIRST_TASK.to_owned()).await.unwrap_err();
        assert!(format!("{error:#}").contains("fixture ordinary request overflow"));
        let thread_id: proteus_contracts::domain::ThreadId =
            serde_json::from_slice(&std::fs::read(root.join("thread.json")).unwrap()).unwrap();
        std::fs::write(
            note_path,
            json!({"session_dir": runtime.session_dir().unwrap(), "thread_id": thread_id})
                .to_string(),
        )
        .unwrap();
    } else {
        let output = runtime
            .run(NEXT_TASK.to_owned())
            .await
            .expect("resumed turn");
        assert_eq!(output.text, FINAL_TEXT);
    }
}

async fn run_child(root: &Path) {
    let output = tokio::time::timeout(
        Duration::from_secs(25),
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "context_overflow::context_overflow_cold_runtime_child",
                "--nocapture",
            ])
            .env(CHILD_ROOT, root)
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("bounded child")
    .expect("child output");
    assert!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_overflow_compacts_after_cold_process_restart() {
    let root = tempfile::tempdir().expect("test root");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let config = configure(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
    )
    .await;
    let server = tokio::spawn(serve(listener));
    run_child(root.path()).await;
    run_child(root.path()).await;
    let requests = server.await.expect("fixture server");
    let note: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("resume.json")).unwrap()).unwrap();
    let session_dir = PathBuf::from(note["session_dir"].as_str().unwrap());
    assert_evidence(&config, root.path(), &session_dir, &requests).await;
}
