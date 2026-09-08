//! Real process workflow + loopback Responses server + cold runtime restart.
//! No live model, credentials or user workspace are involved.
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use proteus_contracts::{
    domain::{ContextRenderMode, new_thread_id},
    model_standard::ContentPart,
};
use proteus_core::core::{
    AgentRuntime, AppConfig, JournalEntry, ModuleCatalog, SessionStore, TurnSettlementStatus,
    WorkflowReplayOptions, replay_workflow,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::Command,
};

#[path = "codex_model_resume/crash_recovery.rs"]
#[cfg(unix)]
mod crash_recovery;
#[path = "codex_model_resume/direct_tool_surface.rs"]
mod direct_tool_surface;
#[path = "codex_model_resume/interruption_recovery.rs"]
mod interruption_recovery;
#[path = "codex_model_resume/model_failure_recovery.rs"]
mod model_failure_recovery;
#[path = "codex_model_resume/partial_sse_recovery.rs"]
mod partial_sse_recovery;
#[path = "codex_model_resume/patch_interception.rs"]
mod patch_interception;
#[path = "codex_model_resume/request_retry.rs"]
mod request_retry;

#[derive(Default)]
struct CapturedEvents(std::sync::Mutex<Vec<proteus_contracts::domain::EventEnvelope>>);

#[async_trait::async_trait]
impl proteus_contracts::contracts::EventSink for CapturedEvents {
    async fn append(&self, event: proteus_contracts::domain::EventEnvelope) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

fn sse_body(response: &Value) -> String {
    let mut body = String::new();
    let mut emit = |kind: &str, data: Value| {
        body.push_str(&format!("event: {kind}\ndata: {data}\n\n"));
    };
    for (index, item) in response["output"].as_array().unwrap().iter().enumerate() {
        if item["type"] != "message" {
            continue;
        }
        let mut added = item.clone();
        added["content"] = json!([]);
        added["status"] = json!("in_progress");
        // Some providers only classify an item when it completes.
        if index == 2 {
            added.as_object_mut().unwrap().remove("phase");
        }
        emit(
            "response.output_item.added",
            json!({"output_index": index, "item": added}),
        );
        for (part_index, part) in item["content"].as_array().unwrap().iter().enumerate() {
            for ch in part["text"].as_str().unwrap().chars() {
                emit(
                    "response.output_text.delta",
                    json!({
                        "output_index": index, "item_id": item["id"], "content_index": part_index, "delta": ch.to_string()
                    }),
                );
            }
        }
        emit(
            "response.output_item.done",
            json!({"output_index": index, "item": item}),
        );
    }
    emit("response.completed", json!({"response": response}));
    body
}

const CHILD_ROOT: &str = "PROTEUS_CODEX_RESUME_TEST_ROOT";

fn message(phase: &str, texts: &[&str]) -> Value {
    json!({"type": "message", "role": "assistant", "phase": phase,
        "content": texts.iter().map(|text| json!({"type": "output_text", "text": text})).collect::<Vec<_>>()})
}

fn responses() -> Vec<Value> {
    vec![
        json!({"status": "completed", "output": [
            {"type": "reasoning", "summary": [{"type": "summary_text", "text": "Проверить файл."}],
                "encrypted_content": "opaque-first+/="},
            message("commentary", &["Смотрю файл.", "\nПроверю содержимое."]),
            {"type": "function_call", "call_id": "call_read", "name": "read_file",
                "arguments": "{ \"path\" : \"probe.txt\" }"}
        ]}),
        json!({"status": "completed", "output": [
            {"type": "reasoning", "summary": [], "encrypted_content": "opaque-second+/="},
            message("commentary", &["Файл прочитан."]),
            message("final_answer", &["Готово.", "\nСодержимое проверено."])
        ]}),
        json!({"status": "completed", "output": [message("final_answer", &["Продолжил."])]}),
    ]
}

async fn read_json_request(socket: &mut tokio::net::TcpStream) -> Value {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 4096];
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0, "request ended before headers");
        bytes.extend_from_slice(&chunk[..n]);
        assert!(bytes.len() < 1_000_000);
        if let Some(offset) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break offset + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
    assert!(headers.starts_with("POST /responses HTTP/1.1"));
    let length: usize = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .expect("content length");
    assert!(length < 1_000_000);
    while bytes.len() < header_end + length {
        let mut chunk = [0; 4096];
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0, "truncated request");
        bytes.extend_from_slice(&chunk[..n]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
}

async fn serve(listener: TcpListener, streaming: bool) -> Vec<Value> {
    let mut requests = Vec::new();
    for (round, mut response) in responses().into_iter().enumerate() {
        // Server output includes provider item ids/status and annotation arrays.
        // The request oracle compares supported semantic fields, not provider ids.
        response["id"] = json!(format!("resp_{round}"));
        response["object"] = json!("response");
        for (index, item) in response["output"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            item["id"] = json!(format!("item_{round}_{index}"));
            item["status"] = json!("completed");
            if let Some(content) = item.get_mut("content").and_then(Value::as_array_mut) {
                for part in content {
                    part["annotations"] = json!([]);
                }
            }
        }
        let (mut socket, _) = listener.accept().await.unwrap();
        requests.push(read_json_request(&mut socket).await);
        let (content_type, body) = if streaming {
            ("text/event-stream", sse_body(&response))
        } else {
            ("application/json", response.to_string())
        };
        socket.write_all(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ).as_bytes()).await.unwrap();
    }
    requests
}

fn config(endpoint: &str, streaming: bool) -> AppConfig {
    serde_json::from_value(json!({
        "active_provider": "fixture",
        "providers": {"fixture": {"provider": "openai", "model": "fixture-model",
            "stream": streaming, "reasoning": {"effort": "high", "summary": true}}},
        "module_config": {"model": {"openai": {"implementation": "openai", "base_url": endpoint, "api_key": "local-fixture-only",
                "http1_only": true, "capabilities": {"supports_reasoning_config": true}}},
            "context": {"codex_context": {"providers": ["project_instructions", "environment"]}}},
        "modules": {"workflow": "coding.codex_loop", "policy": "allow_all", "context": "codex_context"},
        "components": {"fixture": {
            "command": env!("CARGO_BIN_EXE_proteus-reference-worker"),
            "exports": {"model": {"openai": {}}, "workflow": {"coding.codex_loop": {}}, "context": {"codex_context": {}},
                "policy": {"allow_all": {}}, "tool": {"reference.tools": {}}}}},
        "tools": {"enabled": ["read_file"]},
        "runtime": {"model_timeout_ms": 5000, "workflow_timeout_ms": 15000}
    }))
    .unwrap()
}

// The test executable is also a small runtime harness. Each invocation has fresh
// process globals, session writers and component generations, unlike drop/rebuild.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_runtime_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT).map(PathBuf::from) else {
        return;
    };
    let config_path = root.join("config.json");
    let config: AppConfig = serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    let mut builder =
        AgentRuntime::builder(config, root.join("workspace")).with_config_path(Some(&config_path));
    let note_path = root.join("resume.json");
    let (thread_id, prompt) = if note_path.exists() {
        let note: Value = serde_json::from_slice(&std::fs::read(&note_path).unwrap()).unwrap();
        let thread_id = serde_json::from_value(note["thread_id"].clone()).unwrap();
        builder = builder
            .resume_from_session_dir(
                PathBuf::from(note["session_dir"].as_str().unwrap()),
                thread_id,
            )
            .unwrap();
        (thread_id, "Продолжи.")
    } else {
        let thread_id = new_thread_id();
        builder = builder.with_session_ids(proteus_contracts::domain::new_session_id(), thread_id);
        (thread_id, "Прочитай probe.txt.")
    };
    let captured = std::sync::Arc::new(CapturedEvents::default());
    let runtime = builder
        .with_event_sink(captured.clone())
        .build_async()
        .await
        .unwrap();
    let output = runtime.run(prompt.to_owned()).await.unwrap();
    assert!(!output.text.is_empty());
    std::fs::write(
        root.join("events.json"),
        serde_json::to_vec(&*captured.0.lock().unwrap()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        note_path,
        json!({
            "session_dir": runtime.session_dir().unwrap(), "thread_id": thread_id
        })
        .to_string(),
    )
    .unwrap();
}

async fn run_child(root: &Path) {
    let output = tokio::time::timeout(
        Duration::from_secs(25),
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "cold_runtime_child", "--nocapture"])
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

async fn check_resume(streaming: bool) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("workspace")).unwrap();
    std::fs::write(root.path().join("workspace/AGENTS.md"), "Use cargo fmt.\n").unwrap();
    std::fs::write(
        root.path().join("workspace/probe.txt"),
        "fixture-file-content\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = config(
        &format!("http://{}", listener.local_addr().unwrap()),
        streaming,
    );
    std::fs::write(
        root.path().join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let server = tokio::spawn(serve(listener, streaming));
    run_child(root.path()).await;
    let note: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("resume.json")).unwrap()).unwrap();
    let session_dir = PathBuf::from(note["session_dir"].as_str().unwrap());
    let first = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    let first_events: Vec<proteus_contracts::domain::EventEnvelope> =
        serde_json::from_slice(&std::fs::read(root.path().join("events.json")).unwrap()).unwrap();
    let expected_messages: Vec<_> = first
        .history
        .iter()
        .filter(|message| {
            !message.display_text().is_empty()
                && message.role == proteus_contracts::model_standard::MessageRole::Assistant
        })
        .collect();
    for message in &expected_messages {
        let completions: Vec<_> = first_events
            .iter()
            .filter_map(|envelope| match &envelope.event {
                proteus_contracts::domain::Event::AssistantMessageCompleted {
                    message_id,
                    phase,
                    text,
                } if *message_id == message.id => Some((phase, text)),
                _ => None,
            })
            .collect();
        assert!(!completions.is_empty(), "missing presentation completion");
        for (phase, text) in completions {
            assert_eq!(*phase, message.phase);
            assert_eq!(*text, message.display_text());
        }
        let mut streamed = String::new();
        for envelope in &first_events {
            if let proteus_contracts::domain::Event::AssistantTextDelta {
                message_id,
                offset,
                text,
                ..
            } = &envelope.event
                && *message_id == message.id
            {
                assert_eq!(*offset, streamed.len());
                streamed.push_str(text);
            }
        }
        if streaming {
            assert_eq!(streamed, message.display_text());
        } else {
            assert!(streamed.is_empty());
        }
    }
    run_child(root.path()).await;
    let requests = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 3);
    for request in &requests {
        assert_eq!(request["store"], false);
        assert_eq!(request["include"], json!(["reasoning.encrypted_content"]));
    }
    let second_input = requests[1]["input"].as_array().unwrap();
    let resumed_input = requests[2]["input"].as_array().unwrap();
    let first_output = responses()[0]["output"].as_array().unwrap().clone();
    let second_output = responses()[1]["output"].as_array().unwrap().clone();
    // Only compare model-generated items; initial context is runtime-owned.
    let start = second_input
        .iter()
        .position(|item| item["type"] == "reasoning")
        .unwrap();
    assert_eq!(
        &second_input[start..start + first_output.len()],
        &first_output
    );
    let result = &second_input[start + first_output.len()];
    assert_eq!(result["type"], "function_call_output");
    assert_eq!(result["call_id"], "call_read");
    assert!(
        result["output"]
            .as_str()
            .unwrap()
            .contains("fixture-file-content")
    );
    let mut expected = first_output;
    expected.push(result.clone());
    expected.extend(second_output);
    let start = resumed_input
        .iter()
        .position(|item| item["type"] == "reasoning")
        .unwrap();
    assert_eq!(&resumed_input[start..start + expected.len()], &expected);
    assert_eq!(
        resumed_input.len(),
        start + expected.len() + 1,
        "only continuation follows saved history"
    );
    let restored = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    let model_requests: Vec<_> = restored
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::ModelRequestRecorded(recorded) => Some(&recorded.request),
            _ => None,
        })
        .collect();
    assert_eq!(model_requests.len(), requests.len());
    for (canonical, wire) in model_requests.iter().zip(&requests) {
        let chunks: Vec<_> = canonical
            .messages
            .iter()
            .flat_map(|message| message.parts.iter().map(|part| &part.payload))
            .filter_map(|part| match part {
                ContentPart::Context { chunk } => Some(chunk),
                _ => None,
            })
            .collect();
        assert_eq!(
            chunks.len(),
            2,
            "project instructions and environment reach every model round"
        );
        let wire_texts: Vec<_> = wire["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["content"].as_array())
            .flatten()
            .filter_map(|part| part["text"].as_str())
            .collect();
        for chunk in chunks {
            assert_eq!(chunk.render_mode, ContextRenderMode::Verbatim);
            assert!(
                wire_texts.contains(&chunk.content.as_str()),
                "context envelope changed on the process/model boundary"
            );
        }
    }
    assert_eq!(&restored.history[..first.history.len()], &first.history);
    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        root.path().join("workspace"),
        Some(&root.path().join("config.json")),
        session_dir.clone(),
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    for expected in &expected_messages {
        let item = transcript
            .iter()
            .find(|item| item.message_id == Some(expected.id))
            .expect("cold transcript id");
        assert_eq!(item.phase, expected.phase);
        assert_eq!(item.text, expected.display_text());
        assert!(!item.streaming);
    }
    assert!(restored.unsettled_turns.is_empty());
    assert!(restored.interrupted_model_exchanges.is_empty());
    assert!(restored.unresolved_tool_calls.is_empty());
    let turns: Vec<_> = restored
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::TurnSettled(settled) => {
                assert_eq!(settled.status, TurnSettlementStatus::Success);
                record.turn_id
            }
            _ => None,
        })
        .collect();
    assert_eq!(turns.len(), 2);
    for turn_id in turns {
        let catalog = ModuleCatalog::from_config(&config).unwrap();
        let replay = replay_workflow(
            &session_dir,
            &config,
            &catalog,
            WorkflowReplayOptions {
                turn_id: Some(turn_id),
            },
        )
        .await
        .unwrap();
        assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
        assert!(replay.source_journal_unchanged);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_model_state_survives_cold_resume_json() {
    check_resume(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_model_state_survives_cold_resume_sse() {
    check_resume(true).await;
}
