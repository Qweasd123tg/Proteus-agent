//! The same completed mutation survives cancellation, workflow timeout and a
//! later batch infrastructure error, including a cold Core process restart.
use std::{path::Path, sync::Arc, time::Duration};

use async_trait::async_trait;
use proteus_contracts::{
    contracts::{
        ApprovalRequest, ApprovalResponse, ApprovalTransport, CancellationToken, EventSink,
    },
    domain::{Event, EventEnvelope, ToolResult, new_thread_id},
    model_standard::{CanonicalMessage, ContentPart},
};
use proteus_core::core::{
    AgentRuntime, AppConfig, JournalEntry, ModuleCatalog, SessionStore, ToolCallRecordPhase,
    TurnSettlementStatus, WorkflowReplayOptions, replay_workflow,
};
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, net::TcpListener, process::Command, sync::Notify};

use super::direct_tool_surface::{ApprovingTransport, fixture_config};

const CHILD_ROOT: &str = "PROTEUS_INTERRUPTION_RESUME_ROOT";
const COMPLETED_CALL: &str = "call_completed_effect";
const PENDING_CALL: &str = "call_pending_write";
const APPROVAL_FAILURE: &str = "fixture approval transport disconnected";
const CONTINUE: &str = "Продолжай после прерывания.";
const FINAL: &str = "Выполненный шаг сохранён.";

#[derive(Clone, Copy, Debug)]
enum Interruption {
    Cancel,
    Timeout,
    BatchError,
}

impl Interruption {
    fn status(self) -> TurnSettlementStatus {
        match self {
            Self::Cancel => TurnSettlementStatus::Canceled,
            Self::Timeout => TurnSettlementStatus::Timeout,
            Self::BatchError => TurnSettlementStatus::Error,
        }
    }
}

struct ResultBarrier {
    reached: Notify,
    block: bool,
}

#[async_trait]
impl EventSink for ResultBarrier {
    async fn append(&self, envelope: EventEnvelope) -> anyhow::Result<()> {
        if let Event::ToolFinished { result } = envelope.event
            && result.call_id == COMPLETED_CALL
        {
            // This notification follows durable ToolResultRecorded. Blocking
            // here prevents acknowledgement to the batch/workflow controller.
            self.reached.notify_one();
            if self.block {
                std::future::pending::<()>().await;
            }
        }
        Ok(())
    }
}

struct BrokenApproval;

#[async_trait]
impl ApprovalTransport for BrokenApproval {
    fn can_request_approval(&self) -> bool {
        true
    }

    async fn request_approval(&self, request: ApprovalRequest) -> anyhow::Result<ApprovalResponse> {
        assert_eq!(request.call.id, PENDING_CALL);
        // Infrastructure Err, not an ordinary denied/failed ToolResult.
        anyhow::bail!(APPROVAL_FAILURE)
    }
}

fn calls() -> Value {
    json!([
        {"type": "function_call", "call_id": COMPLETED_CALL, "name": "shell",
         "arguments": serde_json::to_string(&json!({
             "command": "printf x >> effects.log; printf done > completed.txt; printf 'effect committed\\n'"
         })).unwrap()},
        {"type": "function_call", "call_id": PENDING_CALL, "name": "write_file",
         "arguments": serde_json::to_string(&json!({"path": "pending.txt", "content": "second effect"})).unwrap()},
    ])
}

async fn serve(listener: TcpListener) -> Value {
    let mut resumed = Value::Null;
    for round in 0..2 {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(25), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let request = super::read_json_request(&mut socket).await;
        if round == 1 {
            resumed = request;
        }
        let output = if round == 0 {
            calls()
        } else {
            json!([{"type": "message", "role": "assistant", "phase": "final_answer",
                "content": [{"type": "output_text", "text": FINAL}]}])
        };
        let body = json!({"status": "completed", "output": output}).to_string();
        socket.write_all(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
        ).as_bytes()).await.unwrap();
    }
    resumed
}

fn results(history: &[CanonicalMessage], call_id: &str) -> Vec<ToolResult> {
    history
        .iter()
        .flat_map(|message| &message.parts)
        .filter_map(|part| match &part.payload {
            ContentPart::ToolResult { result } if result.call_id == call_id => Some(result.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interruption_runtime_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT).map(std::path::PathBuf::from) else {
        return;
    };
    let config_path = root.join("config.json");
    let config: AppConfig = serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    if !root.join("session-path").exists() {
        let mode = match std::fs::read_to_string(root.join("mode")).unwrap().as_str() {
            "cancel" => Interruption::Cancel,
            "timeout" => Interruption::Timeout,
            "batch_error" => Interruption::BatchError,
            mode => panic!("unknown interruption {mode}"),
        };
        run_initial(&root, config, &config_path, mode).await;
        return;
    }
    let session_dir = std::fs::read_to_string(root.join("session-path")).unwrap();
    let runtime = AgentRuntime::builder(config, root.join("workspace"))
        .with_config_path(Some(&config_path))
        .with_approval(Arc::new(ApprovingTransport))
        .resume_from_session_dir(session_dir, new_thread_id())
        .unwrap()
        .build_async()
        .await
        .unwrap();
    assert_eq!(runtime.run(CONTINUE.to_owned()).await.unwrap().text, FINAL);
}

async fn run_child(root: &Path) {
    let output = tokio::time::timeout(
        Duration::from_secs(20),
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "interruption_recovery::interruption_runtime_child",
                "--nocapture",
            ])
            .env(CHILD_ROOT, root)
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "cold child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn run_initial(root: &Path, config: AppConfig, config_path: &Path, mode: Interruption) {
    let barrier = Arc::new(ResultBarrier {
        reached: Notify::new(),
        block: !matches!(mode, Interruption::BatchError),
    });
    let runtime = Arc::new(
        AgentRuntime::builder(config, root.join("workspace"))
            .with_config_path(Some(config_path))
            .with_event_sink(barrier.clone())
            .with_approval(Arc::new(BrokenApproval))
            .build_async()
            .await
            .unwrap(),
    );
    let session_dir = runtime.session_dir().unwrap().to_owned();
    std::fs::write(root.join("session-path"), session_dir.to_str().unwrap()).unwrap();
    let cancel = CancellationToken::new();
    let run = tokio::spawn({
        let runtime = runtime.clone();
        let cancel = cancel.clone();
        async move {
            runtime
                .run_with_cancellation("Выполни два изменения.".to_owned(), cancel)
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(10), barrier.reached.notified())
        .await
        .expect("first tool must reach the committed-result barrier before interruption");
    if matches!(mode, Interruption::Cancel) {
        cancel.cancel();
    }
    let error = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    if matches!(mode, Interruption::BatchError) {
        assert!(format!("{error:#}").contains(APPROVAL_FAILURE), "{error:#}");
    }
    let before = SessionStore::open(session_dir)
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(
        runtime.history().await,
        before.history,
        "warm and cold history disagree"
    );
    let known = results(&before.history, COMPLETED_CALL);
    assert_eq!(known.len(), 1);
    assert!(
        known[0].ok,
        "first tool failed before interruption: {:?}",
        known[0]
    );
    assert!(results(&before.history, PENDING_CALL).is_empty());
}

async fn check_interruption(mode: Interruption) {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = fixture_config(&format!("http://{}", listener.local_addr().unwrap())).await;
    config.module_config.get_mut("policy").unwrap().insert(
        "codex_policy".to_owned(),
        json!({"allow": ["shell"], "ask_before": ["write_file"]}),
    );
    config.runtime.workflow_timeout_ms = 5_000;
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let server = tokio::spawn(serve(listener));
    std::fs::write(
        root.path().join("mode"),
        match mode {
            Interruption::Cancel => "cancel",
            Interruption::Timeout => "timeout",
            Interruption::BatchError => "batch_error",
        },
    )
    .unwrap();
    run_child(root.path()).await;
    let session_dir = std::path::PathBuf::from(
        std::fs::read_to_string(root.path().join("session-path")).unwrap(),
    );
    let store = SessionStore::open(session_dir.clone()).unwrap();
    let before = store.load_projection().unwrap();
    assert!(
        before.unsettled_turns.is_empty(),
        "{mode:?} must settle its root turn"
    );
    let settlements: Vec<_> = before
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::TurnSettled(settled) => Some((record.turn_id.unwrap(), settled)),
            _ => None,
        })
        .collect();
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].1.status, mode.status());
    assert!(settlements[0].1.output.is_none());
    let interrupted_turn = settlements[0].0;
    let known = results(&before.history, COMPLETED_CALL);
    assert_eq!(known.len(), 1);
    assert!(known[0].ok);
    assert!(known[0].text_or_status().contains("effect committed"));
    assert!(results(&before.history, PENDING_CALL).is_empty());
    let second_requested = before.records.iter().filter(|record| matches!(&record.entry,
        JournalEntry::ToolCallRecorded(tool) if tool.call.id == PENDING_CALL && tool.phase == ToolCallRecordPhase::Requested)).count();
    assert_eq!(
        second_requested,
        usize::from(matches!(mode, Interruption::BatchError))
    );
    if matches!(mode, Interruption::BatchError) {
        assert!(before.records.iter().any(|record| matches!(&record.entry,
            JournalEntry::ToolCallRecorded(tool) if tool.call.id == PENDING_CALL && matches!(tool.phase, ToolCallRecordPhase::ApprovalRequested { .. }))));
    }
    run_child(root.path()).await;
    let request = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    let items = request["input"].as_array().unwrap();
    for (index, call_id) in [COMPLETED_CALL, PENDING_CALL].into_iter().enumerate() {
        let calls_in_request: Vec<_> = items
            .iter()
            .filter(|item| item["type"] == "function_call" && item["call_id"] == call_id)
            .collect();
        assert_eq!(calls_in_request.len(), 1);
        assert_eq!(
            calls_in_request[0]["arguments"],
            calls()[index]["arguments"]
        );
        let outputs: Vec<_> = items
            .iter()
            .filter(|item| item["type"] == "function_call_output" && item["call_id"] == call_id)
            .collect();
        assert_eq!(outputs.len(), 1);
        assert_eq!(
            outputs[0]["output"],
            if index == 0 {
                known[0].text_or_status()
            } else {
                "aborted".to_owned()
            }
        );
    }
    assert_eq!(
        std::fs::read_to_string(workspace.join("effects.log")).unwrap(),
        "x"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("completed.txt")).unwrap(),
        "done"
    );
    assert!(!workspace.join("pending.txt").exists());
    let after = store.load_projection().unwrap();
    let settlements: Vec<_> = after
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
        [mode.status(), TurnSettlementStatus::Success]
    );
    let resumed_turn = settlements[1].0;
    assert_eq!(results(&after.history, COMPLETED_CALL), known);
    assert!(
        results(&after.history, PENDING_CALL).is_empty(),
        "prompt-only aborted must not become a durable tool result"
    );
    assert_eq!(
        after
            .records
            .iter()
            .filter(|record| matches!(&record.entry,
        JournalEntry::ToolResultRecorded(tool) if tool.result.call_id == COMPLETED_CALL))
            .count(),
        1
    );
    let app = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        workspace,
        Some(&config_path),
        session_dir.clone(),
    )
    .await
    .unwrap();
    let transcript = app.transcript().await.unwrap();
    for (call_id, status) in [(COMPLETED_CALL, "done"), (PENDING_CALL, "interrupted")] {
        let cards: Vec<_> = transcript
            .iter()
            .filter_map(|message| message.tool.as_ref())
            .filter(|tool| tool.call_id == call_id)
            .collect();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].status, status);
    }
    let catalog = ModuleCatalog::from_config(&config).unwrap();
    let resumed = replay_workflow(
        &session_dir,
        &config,
        &catalog,
        WorkflowReplayOptions {
            turn_id: Some(resumed_turn),
        },
    )
    .await
    .unwrap();
    assert!(
        resumed.comparison.matched,
        "{:?}",
        resumed.comparison.issues
    );
    assert!(resumed.source_journal_unchanged);
    let first = replay_workflow(
        &session_dir,
        &config,
        &catalog,
        WorkflowReplayOptions {
            turn_id: Some(interrupted_turn),
        },
    )
    .await;
    let error = format!(
        "{:#}",
        first.expect_err("external interruption or unresolved approval is not replayable")
    );
    assert!(
        error.contains(match mode {
            Interruption::Cancel => "cannot reproduce external cancellation timing",
            Interruption::Timeout => "cannot reproduce the runtime-owned timeout boundary",
            Interruption::BatchError => "has no recorded resolution",
        }),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_result_survives_cancel_before_workflow_acknowledgement() {
    check_interruption(Interruption::Cancel).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_result_survives_workflow_timeout_before_acknowledgement() {
    check_interruption(Interruption::Timeout).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_result_survives_later_batch_approval_transport_failure() {
    check_interruption(Interruption::BatchError).await;
}
