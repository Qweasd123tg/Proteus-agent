//! End-to-end Codex compaction through process workflow, process compactor and
//! loopback Responses HTTP. The fixture never contacts a live provider.
use std::time::Duration;

use proteus_contracts::{
    domain::new_session_id,
    model_standard::{ContentPart, MessageRole},
};
use proteus_core::core::{AgentRuntime, AppConfig, JournalEntry, SessionStore};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[path = "codex_compaction/compatibility.rs"]
mod compatibility;
#[path = "codex_compaction/replay.rs"]
mod replay;

const COMPACTION_PROMPT: &str =
    include_str!("../../codex-compactor/src/upstream/compact_prompt.md");
const USER_TASK: &str = "Прочитай probe.txt.";
const TOOL_CALL_TEXT: &str = "Сначала прочитаю файл.";
const SUMMARY_TEXT: &str = "Файл прочитан; можно продолжать задачу.";
const FINAL_TEXT: &str = "Готово.";

fn expected_summary() -> String {
    format!(
        "{}\n{SUMMARY_TEXT}",
        include_str!("../../codex-compactor/src/upstream/summary_prefix.md").trim_end()
    )
}

#[derive(Clone)]
enum FixtureReply {
    Json(Value),
    Status { status: u16, body: Value },
    Delayed { delay: Duration, body: Value },
}

fn assistant(phase: &str, text: &str) -> Value {
    json!({
        "type": "message",
        "role": "assistant",
        "phase": phase,
        "content": [{"type": "output_text", "text": text}],
    })
}

fn completed_response(id: &str, output: Vec<Value>, usage: Option<Value>) -> Value {
    let mut response = json!({
        "id": id,
        "object": "response",
        "status": "completed",
        "output": output,
    });
    if let Some(usage) = usage {
        response["usage"] = usage;
    }
    response
}

fn scripted_replies(context_window_once: bool) -> Vec<FixtureReply> {
    let mut replies = vec![FixtureReply::Json(completed_response(
        "turn_tool",
        vec![
            assistant("commentary", TOOL_CALL_TEXT),
            json!({
                "type": "function_call",
                "call_id": "call_read",
                "name": "read_file",
                "arguments": "{\"path\":\"probe.txt\"}",
            }),
        ],
        Some(json!({
            "input_tokens": 12_000,
            "output_tokens": 400,
            "input_tokens_details": {"cached_tokens": 256},
        })),
    ))];
    if context_window_once {
        replies.push(FixtureReply::Status {
            status: 400,
            body: json!({
                "error": {
                    "code": "context_length_exceeded",
                    "message": "fixture context window exceeded"
                }
            }),
        });
    }
    replies.extend([
        FixtureReply::Json(completed_response(
            "compaction",
            vec![assistant("final_answer", SUMMARY_TEXT)],
            Some(json!({"input_tokens": 300, "output_tokens": 40})),
        )),
        FixtureReply::Json(completed_response(
            "turn_final",
            vec![assistant("final_answer", FINAL_TEXT)],
            Some(json!({"input_tokens": 500, "output_tokens": 20})),
        )),
    ]);
    replies
}

async fn read_request(socket: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 4096];
        let read = socket.read(&mut chunk).await.expect("request bytes");
        assert!(read > 0, "request ended before headers");
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() < 1_000_000, "unbounded request headers");
        if let Some(offset) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break offset + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).expect("UTF-8 HTTP headers");
    assert!(headers.starts_with("POST /responses HTTP/1.1"), "{headers}");
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("content length"))
        })
        .expect("content length header");
    assert!(length < 1_000_000, "unbounded request body");
    while bytes.len() < header_end + length {
        let mut chunk = [0; 4096];
        let read = socket.read(&mut chunk).await.expect("request body");
        assert!(read > 0, "truncated request body");
        bytes.extend_from_slice(&chunk[..read]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).expect("Responses JSON")
}

async fn serve(listener: TcpListener, replies: Vec<FixtureReply>) -> Vec<Value> {
    let mut requests = Vec::new();
    for reply in replies {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .expect("bounded fixture accept")
            .expect("fixture accept");
        let request = read_request(&mut socket).await;
        requests.push(request);
        let (status, body) = match reply {
            FixtureReply::Json(body) => (200, body),
            FixtureReply::Status { status, body } => (status, body),
            FixtureReply::Delayed { delay, body } => {
                tokio::time::sleep(delay).await;
                (200, body)
            }
        };
        let status_text = if status == 200 { "OK" } else { "Fixture Error" };
        let body = body.to_string();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .expect("fixture response");
    }
    requests
}

fn config(endpoint: &str) -> AppConfig {
    serde_json::from_value(json!({
        "active_provider": "fixture",
        "providers": {
            "fixture": {
                "provider": "openai",
                "model": "fixture-model",
                "stream": false,
                "reasoning": {"effort": "high", "summary": true}
            }
        },
        "instructions": [{
            "kind": "System",
            "text": "Inherited base instructions.",
            "priority": 100
        }],
        "modules": {
            "workflow": "coding.codex_loop",
            "compactor": "codex",
            "policy": "allow_all",
            "context": "codex_context"
        },
        "module_config": {
            "model": {
                "openai": {
                    "implementation": "openai",
                    "base_url": endpoint,
                    "api_key": "local-fixture-only",
                    "http1_only": true,
                    "prompt_cache": true,
                    "prompt_cache_key": "compaction-fixture-cache",
                    "prompt_cache_retention": "24h",
                    "client_metadata": {"fixture": "codex_compaction"},
                    "capabilities": {"supports_reasoning_config": true}
                }
            },
            "compactor": {"codex": {"trigger_tokens": 1000}},
            "context": {"codex_context": {"providers": ["project_instructions", "environment"]}}
        },
        "components": {
            "fixture": {
                "command": env!("CARGO_BIN_EXE_proteus-reference-worker"),
                "exports": {
                    "model": {"openai": {}},
                    "workflow": {"coding.codex_loop": {}},
                    "compactor": {"codex": {}},
                    "context": {"codex_context": {}},
                    "policy": {"allow_all": {}},
                    "tool": {"reference.tools": {}}
                }
            }
        },
        "tools": {"enabled": ["read_file"]},
        "runtime": {"model_timeout_ms": 5000, "workflow_timeout_ms": 15000}
    }))
    .expect("fixture config")
}

fn text_at(item: &Value) -> &str {
    item["content"][0]["text"]
        .as_str()
        .expect("single text content")
}

fn history_texts(history: &[proteus_contracts::model_standard::CanonicalMessage]) -> Vec<String> {
    history
        .iter()
        .filter_map(|message| (message.role != MessageRole::Tool).then(|| message.display_text()))
        .filter(|text| !text.is_empty())
        .collect()
}

async fn check_codex_compaction(context_window_once: bool) {
    let root = tempfile::tempdir().expect("test root");
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).expect("workspace");
    std::fs::write(workspace.join("AGENTS.md"), "Use cargo fmt.\n").expect("agents");
    std::fs::write(workspace.join("probe.txt"), "fixture file contents\n").expect("probe");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fixture listener");
    let config = config(&format!(
        "http://{}",
        listener.local_addr().expect("fixture address")
    ));
    let config_path = root.path().join("config.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec(&config).expect("config JSON"),
    )
    .expect("config file");
    let server = tokio::spawn(serve(listener, scripted_replies(context_window_once)));

    let runtime = AgentRuntime::builder(config.clone(), workspace.clone())
        .with_config_path(Some(&config_path))
        .with_session_ids(new_session_id(), proteus_contracts::domain::new_thread_id())
        .build_async()
        .await
        .expect("process runtime");
    let output = tokio::time::timeout(Duration::from_secs(20), runtime.run(USER_TASK.to_owned()))
        .await
        .expect("bounded workflow")
        .expect("workflow success");
    assert_eq!(output.text, FINAL_TEXT);
    let session_dir = runtime.session_dir().expect("session dir").to_path_buf();
    drop(runtime);

    let requests = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("fixture completed")
        .expect("fixture task");
    assert_eq!(requests.len(), if context_window_once { 4 } else { 3 });
    let initial = &requests[0];
    let compaction_requests = if context_window_once {
        vec![&requests[1], &requests[2]]
    } else {
        vec![&requests[1]]
    };
    let compact = compaction_requests
        .last()
        .expect("successful compaction request");
    let resumed = requests.last().expect("resumed request");

    for request in &compaction_requests {
        assert_eq!(request["instructions"], initial["instructions"]);
        assert_eq!(request["reasoning"], initial["reasoning"]);
        assert_eq!(request["include"], initial["include"]);
        assert_eq!(request["prompt_cache_key"], initial["prompt_cache_key"]);
        assert_eq!(
            request["prompt_cache_retention"],
            initial["prompt_cache_retention"]
        );
        assert_eq!(request["client_metadata"], initial["client_metadata"]);
        assert_eq!(request["tool_choice"], "none");
        assert!(
            request.get("tools").is_none(),
            "compaction must expose no tools"
        );
        assert_eq!(
            text_at(
                request["input"]
                    .as_array()
                    .expect("compaction input")
                    .last()
                    .expect("prompt")
            ),
            COMPACTION_PROMPT
        );
    }
    assert!(
        compact["input"]
            .as_array()
            .expect("compaction input")
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_read"),
        "successful summary model must see the completed tool result"
    );
    if context_window_once {
        assert!(
            compaction_requests[1]["input"]
                .as_array()
                .expect("retry input")
                .len()
                < compaction_requests[0]["input"]
                    .as_array()
                    .expect("initial input")
                    .len(),
            "context-window recovery must trim history before retrying"
        );
    }

    assert!(
        resumed["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty()),
        "post-compaction request is a normal Codex workflow request"
    );
    assert_eq!(resumed["tool_choice"], "auto");
    let resumed_input = resumed["input"].as_array().expect("resumed input");
    let expected_summary = expected_summary();
    assert_eq!(text_at(&resumed_input[resumed_input.len() - 2]), USER_TASK);
    assert_eq!(
        text_at(resumed_input.last().expect("summary")),
        expected_summary
    );
    assert!(
        resumed_input
            .iter()
            .all(|item| item["type"] != "function_call"
                && item["type"] != "function_call_output"
                && item["content"].as_array().is_none_or(|content| content
                    .iter()
                    .all(|part| part["text"] != TOOL_CALL_TEXT))),
        "old assistant/tool tail must be replaced by the compaction summary"
    );

    let cold = SessionStore::open(session_dir.clone())
        .expect("cold session store")
        .load_projection()
        .expect("cold history projection");
    assert!(cold.unsettled_turns.is_empty());
    assert!(cold.interrupted_model_exchanges.is_empty());
    assert!(cold.unresolved_tool_calls.is_empty());
    assert!(cold.records.iter().any(|record| matches!(
        &record.entry,
        JournalEntry::HistoryMutated(mutation)
            if mutation.compaction.as_ref().is_some_and(|report| report.changed)
    )));
    let history = history_texts(&cold.history);
    assert!(history.iter().any(|text| text == USER_TASK));
    assert!(history.iter().any(|text| text == &expected_summary));
    assert!(history.iter().any(|text| text == FINAL_TEXT));
    assert!(!history.iter().any(|text| text == TOOL_CALL_TEXT));
    assert!(cold.history.iter().all(|message| {
        message.parts.iter().all(|part| {
            !matches!(
                part.payload,
                ContentPart::ToolCall { .. } | ContentPart::ToolResult { .. }
            )
        })
    }));
    assert_eq!(
        cold.records
            .iter()
            .filter(|record| {
                matches!(
                    &record.entry,
                    JournalEntry::ToolResultRecorded(result) if result.result.call_id == "call_read"
                )
            })
            .count(),
        1,
        "compaction recovery must not repeat the already completed tool"
    );
    replay::check(&session_dir, &workspace, &config, context_window_once).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_compaction_uses_a_real_process_model_and_preserves_cold_history() {
    check_codex_compaction(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_compaction_retries_a_typed_context_window_failure_with_shorter_history() {
    check_codex_compaction(true).await;
}
