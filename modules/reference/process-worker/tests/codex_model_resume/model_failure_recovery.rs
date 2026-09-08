use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use proteus_core::core::{
    AgentRuntime, AppConfig, JournalEntry, ModuleCatalog, SessionStore, ToolCallRecordPhase,
    TurnSettlementStatus, WorkflowReplayOptions, replay_workflow,
};
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, net::TcpListener, process::Command};

use super::direct_tool_surface::{ApprovingTransport, fixture_config};

const CHILD_ROOT: &str = "PROTEUS_CODEX_MODEL_FAILURE_TEST_ROOT";
const TARGET_FILE: &str = "written-before-model-failure.txt";
const WRITTEN_CONTENT: &str = "write_file executed exactly once before model failure\n";
const INITIAL_PROMPT: &str = "Запиши контрольный файл через write_file.";
const CONTINUE_PROMPT: &str = "Продолжай после ошибки модели.";

fn tool_response() -> Value {
    json!({
        "id": "failure_recovery_tool",
        "object": "response",
        "status": "completed",
        "output": [{
            "type": "function_call",
            "id": "failure_recovery_write_item",
            "status": "completed",
            "call_id": "call_write_before_failure",
            "name": "write_file",
            "arguments": serde_json::to_string(&json!({
                "path": TARGET_FILE,
                "content": WRITTEN_CONTENT,
            }))
            .unwrap(),
        }],
    })
}

fn success_response() -> Value {
    json!({
        "id": "failure_recovery_success",
        "object": "response",
        "status": "completed",
        "output": [{
            "type": "message",
            "id": "failure_recovery_final",
            "status": "completed",
            "role": "assistant",
            "phase": "final_answer",
            "content": [{
                "type": "output_text",
                "text": "Продолжение увидело выполненную запись.",
                "annotations": [],
            }],
        }],
    })
}

async fn serve(listener: TcpListener) -> Vec<Value> {
    let mut requests = Vec::new();
    for round in 0..3 {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .expect("bounded fixture accept")
            .unwrap();
        requests.push(super::read_json_request(&mut socket).await);
        let (status, body) = match round {
            0 => ("200 OK", tool_response().to_string()),
            1 => (
                "500 Internal Server Error",
                json!({"error": {"message": "fixture model failure", "type": "server_error", "code": "typedOther"}}).to_string(),
            ),
            2 => ("200 OK", success_response().to_string()),
            _ => unreachable!(),
        };
        socket
            .write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }
    requests
}

fn assert_resume_request(failed_round_request: &Value, request: &Value) {
    let actual_results: Vec<_> = failed_round_request["input"]
        .as_array()
        .expect("failed-round Responses input array")
        .iter()
        .filter(|item| {
            item["type"] == "function_call_output" && item["call_id"] == "call_write_before_failure"
        })
        .collect();
    assert_eq!(
        actual_results.len(),
        1,
        "one actual write result before failure"
    );

    let input = request["input"].as_array().expect("Responses input array");
    let tool_calls: Vec<_> = input
        .iter()
        .filter(|item| {
            item["type"] == "function_call"
                && item["call_id"] == "call_write_before_failure"
                && item["name"] == "write_file"
        })
        .collect();
    assert_eq!(
        tool_calls.len(),
        1,
        "executed write call must be restored exactly once"
    );
    assert_eq!(
        tool_calls[0]["arguments"].as_str().unwrap(),
        tool_response()["output"][0]["arguments"].as_str().unwrap(),
    );

    let results: Vec<_> = input
        .iter()
        .filter(|item| {
            item["type"] == "function_call_output" && item["call_id"] == "call_write_before_failure"
        })
        .collect();
    assert_eq!(
        results.len(),
        1,
        "successful write result must be restored exactly once"
    );
    assert_eq!(
        results[0], actual_results[0],
        "resume must preserve the actual successful tool result exactly"
    );
    assert!(
        actual_results[0]["output"]
            .as_str()
            .unwrap()
            .contains("Wrote")
    );

    let call_index = input
        .iter()
        .position(|item| {
            item["call_id"] == "call_write_before_failure" && item["type"] == "function_call"
        })
        .unwrap();
    let result_index = input
        .iter()
        .position(|item| {
            item["call_id"] == "call_write_before_failure" && item["type"] == "function_call_output"
        })
        .unwrap();
    let continue_index = input
        .iter()
        .position(|item| {
            item["role"] == "user"
                && item["content"]
                    .as_array()
                    .is_some_and(|parts| parts.iter().any(|part| part["text"] == CONTINUE_PROMPT))
        })
        .expect("continuation user message reaches resumed model request");
    assert!(call_index < result_index && result_index < continue_index);
}

async fn assert_canonical_session(config: &AppConfig, session_dir: &Path) {
    let projection = SessionStore::open(session_dir.to_owned())
        .unwrap()
        .load_projection()
        .unwrap();
    let requested = projection
        .records
        .iter()
        .filter(|record| {
            matches!(
                &record.entry,
                JournalEntry::ToolCallRecorded(tool)
                    if tool.phase == ToolCallRecordPhase::Requested
                        && tool.call.id == "call_write_before_failure"
            )
        })
        .count();
    assert_eq!(requested, 1);
    let results = projection
        .records
        .iter()
        .filter(|record| {
            matches!(
                &record.entry,
                JournalEntry::ToolResultRecorded(result)
                    if result.result.call_id == "call_write_before_failure"
            )
        })
        .count();
    assert_eq!(results, 1);
    assert!(projection.unresolved_tool_calls.is_empty());

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

    for (turn_id, status) in settlements {
        let catalog = ModuleCatalog::from_config(config).unwrap();
        let replay = replay_workflow(
            session_dir,
            config,
            &catalog,
            WorkflowReplayOptions {
                turn_id: Some(turn_id),
            },
        )
        .await;
        match status {
            TurnSettlementStatus::Success => {
                let replay = replay.unwrap();
                assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
                assert!(replay.source_journal_unchanged);
            }
            TurnSettlementStatus::Error => {
                let replay = replay.unwrap();
                assert_eq!(replay.recorded.status, TurnSettlementStatus::Error);
                assert_eq!(replay.replay.status, TurnSettlementStatus::Error);
                assert!(
                    replay.comparison.matched,
                    "failed-turn replay diverged: {:?}",
                    replay.comparison.issues,
                );
                assert!(replay.source_journal_unchanged);
            }
            _ => unreachable!(),
        }
    }
}

async fn configure(root: &Path, endpoint: &str) -> AppConfig {
    std::fs::create_dir(root.join("workspace")).unwrap();
    std::fs::write(
        root.join("workspace/AGENTS.md"),
        "Use the configured tools.\n",
    )
    .unwrap();
    let config = fixture_config(endpoint).await;
    std::fs::write(
        root.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    config
}

async fn assert_cold_history(config: AppConfig, root: &Path, session_dir: &Path) {
    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config,
        root.join("workspace"),
        Some(&root.join("config.json")),
        session_dir.to_owned(),
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    assert!(transcript.iter().any(|item| item.text == INITIAL_PROMPT));
    assert!(transcript.iter().any(|item| item.text == CONTINUE_PROMPT));
    assert!(
        transcript
            .iter()
            .any(|item| item.text.contains("Продолжение увидело"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_tool_survives_model_failure_and_warm_continuation() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = configure(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
    )
    .await;
    let server = tokio::spawn(serve(listener));
    let config_path = root.path().join("config.json");

    let runtime = AgentRuntime::builder(config.clone(), root.path().join("workspace"))
        .with_config_path(Some(&config_path))
        .with_approval(Arc::new(ApprovingTransport))
        .build_async()
        .await
        .unwrap();
    let error = runtime.run(INITIAL_PROMPT.to_owned()).await.unwrap_err();
    assert!(format!("{error:#}").contains("fixture model failure"));
    assert_eq!(
        std::fs::read_to_string(root.path().join("workspace").join(TARGET_FILE)).unwrap(),
        WRITTEN_CONTENT
    );
    runtime.run(CONTINUE_PROMPT.to_owned()).await.unwrap();

    let requests = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 3);
    assert_resume_request(&requests[1], &requests[2]);
    let session_dir = runtime.session_dir().unwrap();
    assert_canonical_session(&config, &session_dir).await;
    assert_cold_history(config, root.path(), &session_dir).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_failure_cold_runtime_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT).map(PathBuf::from) else {
        return;
    };
    let config_path = root.join("config.json");
    let config: AppConfig = serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    let note_path = root.join("resume.json");
    let mut builder =
        AgentRuntime::builder(config, root.join("workspace")).with_config_path(Some(&config_path));
    let first = !note_path.exists();
    if first {
        let thread_id = proteus_contracts::domain::new_thread_id();
        builder = builder.with_session_ids(proteus_contracts::domain::new_session_id(), thread_id);
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
            .unwrap();
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
        assert!(format!("{error:#}").contains("fixture model failure"));
        let thread_id: proteus_contracts::domain::ThreadId =
            serde_json::from_slice(&std::fs::read(root.join("thread.json")).unwrap()).unwrap();
        std::fs::write(
            note_path,
            json!({"session_dir": runtime.session_dir().unwrap(), "thread_id": thread_id})
                .to_string(),
        )
        .unwrap();
    } else {
        assert!(result.unwrap().text.contains("Продолжение увидело"));
    }
}

async fn run_child(root: &Path) {
    let output = tokio::time::timeout(
        Duration::from_secs(25),
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "model_failure_recovery::model_failure_cold_runtime_child",
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
async fn completed_tool_survives_model_failure_and_cold_process_restart() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = configure(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
    )
    .await;
    let server = tokio::spawn(serve(listener));

    run_child(root.path()).await;
    assert_eq!(
        std::fs::read_to_string(root.path().join("workspace").join(TARGET_FILE)).unwrap(),
        WRITTEN_CONTENT
    );
    run_child(root.path()).await;
    let requests = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 3);
    assert_resume_request(&requests[1], &requests[2]);

    let note: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("resume.json")).unwrap()).unwrap();
    let session_dir = PathBuf::from(note["session_dir"].as_str().unwrap());
    assert_canonical_session(&config, &session_dir).await;
    assert_cold_history(config, root.path(), &session_dir).await;
}
