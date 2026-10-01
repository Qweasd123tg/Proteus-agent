use std::{path::Path, sync::Arc, time::Duration};

use async_trait::async_trait;
use proteus_contracts::{
    contracts::EventSink,
    domain::{Event, EventEnvelope},
    model_standard::ContentPart,
};
use proteus_core::core::{
    AgentRuntime, AppConfig, JournalEntry, ModuleCatalog, SessionStore, WorkflowReplayOptions,
    replay_workflow,
};
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, net::TcpListener, process::Command};

use super::direct_tool_surface::{ApprovingTransport, fixture_config};

const CHILD_ROOT: &str = "PROTEUS_CRASH_RECOVERY_ROOT";
const CALL_ID: &str = "call_effect_before_crash";

struct StopAfterResult(std::path::PathBuf);

#[async_trait]
impl EventSink for StopAfterResult {
    async fn append(&self, envelope: EventEnvelope) -> anyhow::Result<()> {
        if let Event::ToolFinished { result } = envelope.event
            && result.call_id == CALL_ID
        {
            // BoundTools has committed the result, but its caller has not received it.
            std::fs::write(&self.0, "result committed")?;
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_runtime_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT).map(std::path::PathBuf::from) else {
        return;
    };
    let path = root.join("config.json");
    let config: AppConfig = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut builder = AgentRuntime::builder(config, root.join("workspace"))
        .with_config_path(Some(&path))
        .with_approval(Arc::new(ApprovingTransport));
    if root.join("after-result").exists() {
        builder = builder.with_event_sink(Arc::new(StopAfterResult(root.join("ready"))));
    }
    let runtime = builder.build_async().await.unwrap();
    std::fs::write(
        root.join("session-path"),
        runtime.session_dir().unwrap().to_str().unwrap(),
    )
    .unwrap();
    runtime.run("Измени файлы.".to_owned()).await.unwrap();
    panic!("parent must kill the runtime at the barrier");
}

async fn reply(socket: &mut tokio::net::TcpStream, output: Value) {
    let body = json!({"status": "completed", "output": output}).to_string();
    socket.write_all(format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
    ).as_bytes()).await.unwrap();
}

async fn wait_for_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("barrier not reached: {}", path.display()));
}

async fn crash_and_resume(after_result: bool, freeform: bool) {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = fixture_config(&format!("http://{}", listener.local_addr().unwrap())).await;
    let config_path = root.path().join("config.json");
    if after_result {
        std::fs::write(root.path().join("after-result"), "").unwrap();
    }
    // The effect is deliberately non-idempotent. In the first window the shell
    // waits after the write; no tool response can reach Core before its death.
    let gate = tokio::net::UnixListener::bind(workspace.join("crash-gate.sock")).unwrap();
    let command = if after_result {
        "printf x >> effects.log; printf a > a.txt; printf b > b.txt; printf c > c.txt".to_owned()
    } else {
        "python3 -c 'from pathlib import Path; import socket; f=open(\"effects.log\",\"a\"); f.write(\"x\"); f.close(); [Path(n+\".txt\").write_text(n) for n in \"abc\"]; s=socket.socket(socket.AF_UNIX); s.connect(\"crash-gate.sock\"); s.recv(1)'".to_owned()
    };
    if freeform {
        // The local Responses fixture supports custom tools; the packaged
        // proxy deliberately declares only the function surface.
        config
            .module_config
            .get_mut("model")
            .unwrap()
            .get_mut("openai")
            .unwrap()["capabilities"]["supports_freeform_tools"] = json!(true);
        config.tools.configured.push(
            serde_json::from_value(json!({
                "name": "crash_probe",
                "description": "Write the fixture files from a custom tool invocation.",
                "surface": {"kind": "freeform", "format": {"type": "grammar", "syntax": "lark", "definition": "start: /[a-z ]+/"}},
                "safety": "RunsCommands",
                "timeout_ms": 20000,
                "executor": {"kind": "process", "command": "/bin/sh", "args": ["-c", command]}
            }))
            .unwrap(),
        );
    }
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let call_type = if freeform {
        "custom_tool_call"
    } else {
        "function_call"
    };
    let output_type = if freeform {
        "custom_tool_call_output"
    } else {
        "function_call_output"
    };
    let model = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = super::read_json_request(&mut socket).await;
        let call = if freeform {
            assert!(
                request["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["type"] == "custom" && tool["name"] == "crash_probe")
            );
            json!({"type": call_type, "call_id": CALL_ID, "name": "crash_probe", "input": "change files"})
        } else {
            json!({"type": call_type, "call_id": CALL_ID, "name": "shell",
                "arguments": serde_json::to_string(&json!({"command": command})).unwrap()})
        };
        reply(&mut socket, json!([call])).await;
        let (mut socket, _) = listener.accept().await.unwrap();
        let resumed_request = super::read_json_request(&mut socket).await;
        reply(
            &mut socket,
            json!([{
                "type": "message", "role": "assistant", "phase": "final_answer",
                "content": [{"type": "output_text", "text": "Продолжение."}]
            }]),
        )
        .await;
        resumed_request
    });
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_recovery::crash_runtime_child",
            "--nocapture",
        ])
        .env(CHILD_ROOT, root.path())
        .env("NO_PROXY", "127.0.0.1")
        .env("no_proxy", "127.0.0.1")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let barrier = if after_result {
        wait_for_file(&root.path().join("ready")).await;
        None
    } else {
        Some(
            tokio::time::timeout(Duration::from_secs(15), gate.accept())
                .await
                .expect("side effect reached the barrier")
                .unwrap()
                .0,
        )
    };
    child.kill().await.unwrap();
    assert!(!child.wait().await.unwrap().success());
    drop(barrier);
    for name in ["a", "b", "c"] {
        assert_eq!(
            std::fs::read_to_string(workspace.join(format!("{name}.txt"))).unwrap(),
            name
        );
    }
    let session_path = std::fs::read_to_string(root.path().join("session-path")).unwrap();
    let store = SessionStore::open(session_path.into()).unwrap();
    let projection = store.load_projection().unwrap();
    assert_eq!(projection.unsettled_turns.len(), 1);
    assert_eq!(projection.unresolved_tool_calls.is_empty(), after_result);
    let actual_result = projection
        .records
        .iter()
        .find_map(|record| match &record.entry {
            JournalEntry::ToolResultRecorded(tool) if tool.result.call_id == CALL_ID => {
                Some(tool.result.clone())
            }
            _ => None,
        });
    assert_eq!(actual_result.is_some(), after_result);

    let runtime = AgentRuntime::builder(config.clone(), workspace.clone())
        .with_config_path(Some(&config_path))
        .with_approval(Arc::new(ApprovingTransport))
        .resume_from_session_dir(
            store.session_dir().to_owned(),
            proteus_contracts::domain::new_thread_id(),
        )
        .unwrap()
        .build_async()
        .await
        .unwrap();
    runtime.run("Продолжай.".to_owned()).await.unwrap();
    let request = tokio::time::timeout(Duration::from_secs(10), model)
        .await
        .unwrap()
        .unwrap();
    let items = request["input"].as_array().unwrap();
    let calls: Vec<_> = items
        .iter()
        .filter(|item| item["type"] == call_type && item["call_id"] == CALL_ID)
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "cold resume lost the call whose side effect happened"
    );
    let results: Vec<_> = items
        .iter()
        .filter(|item| item["type"] == output_type && item["call_id"] == CALL_ID)
        .collect();
    assert_eq!(
        results.len(),
        1,
        "resume must provide the known result or prompt-only aborted output"
    );
    if after_result {
        assert_eq!(
            results[0]["output"].as_str().unwrap(),
            actual_result.unwrap().text_or_status()
        );
    } else {
        assert_eq!(results[0]["output"], "aborted");
    }
    assert_eq!(
        std::fs::read_to_string(workspace.join("effects.log")).unwrap(),
        "x"
    );
    let history = store.load_messages().unwrap();
    let stored_results = history.iter().flat_map(|message| &message.parts).filter(|part| matches!(&part.payload, ContentPart::ToolResult { result } if result.call_id == CALL_ID)).count();
    assert_eq!(
        stored_results,
        usize::from(after_result),
        "unknown effects must not become fabricated durable results"
    );
    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config,
        workspace,
        Some(&config_path),
        store.session_dir().to_owned(),
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    let cards: Vec<_> = transcript
        .iter()
        .filter_map(|message| message.tool.as_ref())
        .filter(|tool| tool.call_id == CALL_ID)
        .collect();
    assert_eq!(cards.len(), 1);
    assert_eq!(
        cards[0].status,
        if after_result { "done" } else { "interrupted" }
    );
    let projection = store.load_projection().unwrap();
    let resumed_turn = projection
        .records
        .iter()
        .find_map(|record| match &record.entry {
            JournalEntry::TurnSettled(_) => record.turn_id,
            _ => None,
        })
        .unwrap();
    let config: AppConfig = serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    let catalog = ModuleCatalog::from_config(&config).unwrap();
    let replay = replay_workflow(
        store.session_dir(),
        &config,
        &catalog,
        WorkflowReplayOptions {
            turn_id: Some(resumed_turn),
        },
    )
    .await
    .unwrap();
    assert!(
        replay.comparison.matched,
        "continuation replay diverged: {:?}",
        replay.comparison.issues
    );
    let crash_turn = projection.unsettled_turns[0];
    assert!(
        replay_workflow(
            store.session_dir(),
            &config,
            &catalog,
            WorkflowReplayOptions {
                turn_id: Some(crash_turn)
            }
        )
        .await
        .is_err(),
        "a crashed turn is not a replayable terminal outcome"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn effect_without_result_survives_crash_as_unknown() {
    crash_and_resume(false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_result_survives_crash_before_workflow_acknowledgement() {
    crash_and_resume(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_effect_without_result_gets_prompt_only_aborted_on_cold_resume() {
    crash_and_resume(false, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_custom_result_survives_crash_without_a_synthetic_duplicate() {
    crash_and_resume(true, true).await;
}
