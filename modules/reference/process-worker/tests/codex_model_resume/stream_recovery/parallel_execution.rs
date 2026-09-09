//! Real component barriers prove overlap, an exclusive fence and ordered drain.
//! The model stream remains open until tools finish, or disconnects mid-flight.
use std::{path::PathBuf, sync::Mutex};

use async_trait::async_trait;
use proteus_contracts::contracts::{ApprovalRequest, ApprovalResponse, ApprovalTransport};
use proteus_core::core::{AppConfig, HistoryMutationKind};

use super::*;

#[path = "parallel_execution/cancellation.rs"]
mod cancellation;

const LABELS: [&str; 4] = ["a", "b", "write", "c"];
const QUEUED: &str = "Все четыре вызова переданы на исполнение.";

struct ApprovalProbe {
    deny: bool,
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl ApprovalTransport for ApprovalProbe {
    fn can_request_approval(&self) -> bool {
        true
    }

    async fn request_approval(&self, request: ApprovalRequest) -> anyhow::Result<ApprovalResponse> {
        self.calls.lock().unwrap().push(request.call.id.clone());
        Ok(if self.deny {
            ApprovalResponse::deny("fixture denial")
        } else {
            ApprovalResponse::approve()
        })
    }
}

async fn config(root: &Path, listener: &TcpListener) -> AppConfig {
    let mut config = configure(
        root,
        &format!("http://{}", listener.local_addr().unwrap()),
        1,
    )
    .await;
    config.tools.enabled = vec!["parallel_probe".into(), "exclusive_probe".into()];
    config.module_config.get_mut("policy").unwrap().insert(
        "codex_policy".into(),
        json!({"allow": ["parallel_probe"], "ask_before": ["exclusive_probe"]}),
    );
    config.components.insert("stream-tools-component".into(), serde_json::from_value(json!({
        "command": "python3", "args": ["-B", Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stream_tools.py")],
        "handshake_timeout_ms": 3000,
        "exports": {"tool": {"stream-tools": {"timeout_ms": 12000}}},
    })).unwrap());
    config
}

fn items(directory: &Path) -> Vec<Value> {
    let mut items = LABELS
        .into_iter()
        .map(|label| {
            json!({
                "type": "function_call", "id": format!("item_{label}"), "status": "completed",
                "call_id": format!("call_{label}"),
                "name": if label == "write" { "exclusive_probe" } else { "parallel_probe" },
                "arguments": json!({"directory": directory, "label": label}).to_string(),
            })
        })
        .collect::<Vec<_>>();
    items.push(
        json!({"type":"message", "id":"queued_item", "status":"completed",
        "role":"assistant", "phase":"commentary",
        "content":[{"type":"output_text", "text":QUEUED, "annotations":[]}]}),
    );
    items
}

async fn wait_for(description: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("fixture barrier: {description}"));
}

fn has_result(session: &Path, label: &str) -> bool {
    SessionStore::open(session.to_owned()).unwrap().load_projection().unwrap().records.iter().any(|record| {
        matches!(&record.entry, JournalEntry::ToolResultRecorded(tool) if tool.result.call_id == format!("call_{label}"))
    })
}

async fn open_stream(
    listener: &TcpListener,
    directory: &Path,
    session: &Path,
) -> (tokio::net::TcpStream, Value) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let request = read_json_request(&mut socket).await;
    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    for (index, item) in items(directory).iter().enumerate() {
        socket
            .write_all(
                format!(
                    "event: response.output_item.done\ndata: {}\n\n",
                    json!({"output_index": index, "item": item})
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }
    wait_for("two tools execute while SSE stays open", || {
        directory.join("started-a").exists() && directory.join("started-b").exists()
    })
    .await;
    // An acknowledged checkpoint after all calls makes the fence assertion
    // deterministic: the model/workflow has consumed the queued calls.
    wait_for("all calls checkpointed", || {
        SessionStore::open(session.to_owned())
            .unwrap()
            .load_projection()
            .unwrap()
            .records
            .iter()
            .any(|record| {
                matches!(&record.entry, JournalEntry::HistoryMutated(mutation)
                if mutation.mutation == HistoryMutationKind::Checkpoint
                    && mutation.messages.iter().any(|message| message.display_text() == QUEUED))
            })
    })
    .await;
    assert!(!directory.join("started-write").exists());
    assert!(!directory.join("started-c").exists());
    (socket, request)
}

fn release(directory: &Path, label: &str) {
    std::fs::write(directory.join(format!("release-{label}")), "release").unwrap();
}

async fn serve(
    listener: TcpListener,
    directory: PathBuf,
    session: PathBuf,
    disconnect: bool,
    deny: bool,
) -> Value {
    let (mut socket, _) = open_stream(&listener, &directory, &session).await;
    if disconnect {
        socket.shutdown().await.unwrap();
    }
    release(&directory, "b");
    wait_for("b commits while a is blocked", || has_result(&session, "b")).await;
    assert!(!has_result(&session, "a"));
    assert!(!directory.join("started-write").exists());
    assert!(!directory.join("started-c").exists());
    release(&directory, "a");
    if !deny {
        wait_for("exclusive call starts after both readers", || {
            directory.join("started-write").exists()
        })
        .await;
        assert!(has_result(&session, "a"));
        assert!(!directory.join("started-c").exists());
        release(&directory, "write");
    }
    wait_for("exclusive result settles", || has_result(&session, "write")).await;
    wait_for("last reader starts after exclusive call", || {
        directory.join("started-c").exists()
    })
    .await;
    release(&directory, "c");
    wait_for("last reader result commits", || has_result(&session, "c")).await;
    if !disconnect {
        socket
            .write_all(
                sse_body(&response(json!(items(&directory)), "parallel_response")).as_bytes(),
            )
            .await
            .unwrap();
        socket.shutdown().await.unwrap();
    }
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let next = read_json_request(&mut socket).await;
    write_sse(&mut socket, &sse_body(&final_response())).await;
    next
}

async fn check(disconnect: bool, deny: bool) {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = config(root.path(), &listener).await;
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let directory = root.path().join("workspace");
    let approval = Arc::new(ApprovalProbe {
        deny,
        calls: Mutex::new(Vec::new()),
    });
    let runtime = AgentRuntime::builder(config.clone(), directory.clone())
        .with_config_path(Some(&config_path))
        .with_approval(approval.clone())
        .build_async()
        .await
        .unwrap();
    let session = runtime.session_dir().unwrap().to_owned();
    let server = tokio::spawn(serve(
        listener,
        directory.clone(),
        session.clone(),
        disconnect,
        deny,
    ));
    assert_eq!(runtime.run(PROMPT.to_owned()).await.unwrap().text, FINAL);
    let next = server.await.unwrap();
    let output = next["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .collect::<Vec<_>>();
    assert_eq!(
        output
            .iter()
            .map(|item| item["call_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["call_a", "call_b", "call_write", "call_c"]
    );
    assert_eq!(approval.calls.lock().unwrap().as_slice(), ["call_write"]);
    let effect = directory.join("effects.log");
    if deny {
        assert!(!effect.exists());
        assert!(!directory.join("started-write").exists());
    } else {
        assert_eq!(std::fs::read_to_string(&effect).unwrap(), "write\n");
    }
    let projection = SessionStore::open(session.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(projection.history, runtime.history().await);
    let result_order = projection
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::ToolResultRecorded(tool) => Some(tool.result.call_id.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(result_order, ["call_b", "call_a", "call_write", "call_c"]);
    drop(runtime);
    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        directory.clone(),
        Some(&config_path),
        session.clone(),
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    for label in LABELS {
        assert_eq!(
            transcript
                .iter()
                .filter_map(|item| item.tool.as_ref())
                .filter(|tool| tool.call_id == format!("call_{label}")
                    && tool.status
                        == if deny && label == "write" {
                            "failed"
                        } else {
                            "done"
                        })
                .count(),
            1
        );
    }
    drop(cold);
    if effect.exists() {
        std::fs::remove_file(&effect).unwrap();
    }
    let replay = replay_workflow(
        &session,
        &config,
        &ModuleCatalog::from_config(&config).unwrap(),
        WorkflowReplayOptions::default(),
    )
    .await
    .unwrap();
    assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
    assert!(replay.source_journal_unchanged);
    assert!(!effect.exists(), "replay must not repeat the effect");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streamed_readers_overlap_and_results_drain_in_model_order() {
    check(false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_drains_in_flight_calls_before_retry() {
    check(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn denied_exclusive_call_releases_the_next_reader() {
    check(false, true).await;
}
