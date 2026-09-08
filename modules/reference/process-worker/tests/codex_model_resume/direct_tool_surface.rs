use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use proteus_contracts::contracts::{ApprovalRequest, ApprovalResponse, ApprovalTransport};
use proteus_core::core::{
    AgentRuntime, AppConfig, JournalEntry, ModuleCatalog, SessionStore, ToolCallRecordPhase,
    TurnSettlementStatus, WorkflowReplayOptions, replay_workflow,
};
use proteus_core::process_adapters::ProcessComponentConfig;
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, net::TcpListener, process::Command};

const CHILD_ROOT: &str = "PROTEUS_CODEX_DIRECT_TOOLS_TEST_ROOT";
const WRITTEN_CONTENT: &str = "direct Codex profile tool call\n";

struct ApprovingTransport;

#[async_trait]
impl ApprovalTransport for ApprovingTransport {
    fn can_request_approval(&self) -> bool {
        true
    }

    async fn request_approval(
        &self,
        _request: ApprovalRequest,
    ) -> anyhow::Result<ApprovalResponse> {
        Ok(ApprovalResponse::approve())
    }
}

fn workspace_file(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(path)
}

fn response(output: Value, round: usize) -> Value {
    json!({
        "id": format!("direct_tools_{round}"),
        "object": "response",
        "status": "completed",
        "output": output,
    })
}

fn responses() -> Vec<Value> {
    vec![
        response(
            json!([{
                "type": "function_call",
                "id": "write_item",
                "status": "completed",
                "call_id": "call_write",
                "name": "write_file",
                "arguments": serde_json::to_string(&json!({
                    "path": "created-by-direct-tool.txt",
                    "content": WRITTEN_CONTENT,
                }))
                .unwrap(),
            }]),
            0,
        ),
        response(
            json!([{
                "type": "message",
                "id": "message_written",
                "status": "completed",
                "role": "assistant",
                "phase": "final_answer",
                "content": [{
                    "type": "output_text",
                    "text": "Файл записан напрямую.",
                    "annotations": [],
                }],
            }]),
            1,
        ),
        response(
            json!([{
                "type": "message",
                "id": "message_resumed",
                "status": "completed",
                "role": "assistant",
                "phase": "final_answer",
                "content": [{
                    "type": "output_text",
                    "text": "История восстановлена.",
                    "annotations": [],
                }],
            }]),
            2,
        ),
    ]
}

async fn serve(listener: TcpListener) -> Vec<Value> {
    let mut requests = Vec::new();
    for fixture in responses() {
        let (mut socket, _) = listener.accept().await.unwrap();
        requests.push(super::read_json_request(&mut socket).await);
        let body = fixture.to_string();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }
    requests
}

async fn fixture_config(endpoint: &str) -> AppConfig {
    let profile = workspace_file("configs/codex.config.toml");
    let mut config = AppConfig::load(Some(&profile))
        .await
        .expect("tracked Codex profile loads");

    config
        .providers
        .get_mut(&config.active_provider)
        .expect("active Codex provider")
        .stream = false;
    let model = config
        .module_config
        .get_mut("model")
        .and_then(|models| models.get_mut("openai"))
        .and_then(Value::as_object_mut)
        .expect("OpenAI process export config");
    model.remove("api_key_file");
    model.remove("api_key_json_key");
    model.remove("base_url_file");
    model.remove("base_url_json_key");
    model.insert("api_key".to_owned(), json!("local-fixture-only"));
    model.insert("base_url".to_owned(), json!(endpoint));
    model.insert("http1_only".to_owned(), json!(true));

    for component in config.components.values_mut() {
        let mut value = serde_json::to_value(&*component).expect("component config value");
        value["command"] = json!(env!("CARGO_BIN_EXE_proteus-reference-worker"));
        *component = serde_json::from_value::<ProcessComponentConfig>(value)
            .expect("fixture worker command");
    }
    config.runtime.model_timeout_ms = 5_000;
    config.runtime.workflow_timeout_ms = 15_000;
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_surface_cold_runtime_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT).map(PathBuf::from) else {
        return;
    };
    let config_path = root.join("config.json");
    let config: AppConfig = serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    let note_path = root.join("resume.json");
    let mut builder =
        AgentRuntime::builder(config, root.join("workspace")).with_config_path(Some(&config_path));
    let prompt = if note_path.exists() {
        let note: Value = serde_json::from_slice(&std::fs::read(&note_path).unwrap()).unwrap();
        let thread_id = serde_json::from_value(note["thread_id"].clone()).unwrap();
        builder = builder
            .resume_from_session_dir(
                PathBuf::from(note["session_dir"].as_str().unwrap()),
                thread_id,
            )
            .unwrap();
        "Продолжи по сохранённой истории."
    } else {
        let thread_id = proteus_contracts::domain::new_thread_id();
        builder = builder.with_session_ids(proteus_contracts::domain::new_session_id(), thread_id);
        std::fs::write(
            root.join("thread.json"),
            serde_json::to_vec(&thread_id).unwrap(),
        )
        .unwrap();
        "Создай файл через write_file."
    };
    let runtime = builder
        .with_approval(Arc::new(ApprovingTransport))
        .build_async()
        .await
        .unwrap();
    let output = runtime.run(prompt.to_owned()).await.unwrap();
    assert!(!output.text.is_empty());
    let thread_id: proteus_contracts::domain::ThreadId =
        serde_json::from_slice(&std::fs::read(root.join("thread.json")).unwrap()).unwrap();
    std::fs::write(
        note_path,
        json!({"session_dir": runtime.session_dir().unwrap(), "thread_id": thread_id}).to_string(),
    )
    .unwrap();
}

async fn run_child(root: &Path) {
    let output = tokio::time::timeout(
        Duration::from_secs(25),
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "direct_tool_surface::direct_surface_cold_runtime_child",
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

fn request_tool_names(request: &Value) -> HashSet<&str> {
    request["tools"]
        .as_array()
        .expect("Responses tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tracked_codex_profile_exposes_and_executes_policy_visible_tools_directly() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("workspace")).unwrap();
    std::fs::write(
        root.path().join("workspace/AGENTS.md"),
        "Use direct tools.\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = fixture_config(&format!("http://{}", listener.local_addr().unwrap())).await;
    assert!(config.modules.tool_exposure.is_none());
    let configured_tools = config.tools.enabled.clone();
    std::fs::write(
        root.path().join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();

    let server = tokio::spawn(serve(listener));
    run_child(root.path()).await;
    assert_eq!(
        std::fs::read_to_string(root.path().join("workspace/created-by-direct-tool.txt")).unwrap(),
        WRITTEN_CONTENT
    );
    run_child(root.path()).await;
    let requests = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 3);

    for request in &requests {
        let names = request_tool_names(request);
        for expected in &configured_tools {
            assert!(
                names.contains(expected.as_str()),
                "configured policy-visible tool {expected} missing from direct surface; got {names:?}"
            );
        }
        for expected in ["grep", "git_diff", "write_file"] {
            assert!(
                names.contains(expected),
                "{expected} missing from direct tool surface"
            );
        }
        for forbidden in [
            "proteus_tool_search",
            "proteus_tool_describe",
            "proteus_tool_call",
        ] {
            assert!(!names.contains(forbidden), "unexpected tool {forbidden}");
        }
        let serialized = request.to_string();
        assert!(!serialized.contains("You do not receive the full tool catalog up front"));
        assert!(!serialized.contains("call proteus_tool_search"));
    }
    let result = requests[1]["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .expect("write_file result reaches the following model request");
    assert_eq!(result["call_id"], "call_write");
    assert!(result["output"].as_str().unwrap().contains("Wrote"));

    let note: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("resume.json")).unwrap()).unwrap();
    let session_dir = PathBuf::from(note["session_dir"].as_str().unwrap());
    let projection = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    let requested: Vec<_> = projection
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::ToolCallRecorded(tool)
                if tool.phase == ToolCallRecordPhase::Requested
                    && tool.call.name == "write_file" =>
            {
                Some(&tool.call.id)
            }
            _ => None,
        })
        .collect();
    assert_eq!(requested, ["call_write"]);
    let results = projection
        .records
        .iter()
        .filter(|record| {
            matches!(
                &record.entry,
                JournalEntry::ToolResultRecorded(result) if result.result.call_id == "call_write"
            )
        })
        .count();
    assert_eq!(results, 1, "one canonical result for the one execution");
    assert!(projection.unresolved_tool_calls.is_empty());

    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        root.path().join("workspace"),
        Some(&root.path().join("config.json")),
        session_dir.clone(),
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    assert!(
        transcript
            .iter()
            .any(|item| item.text.contains("Файл записан напрямую."))
    );
    assert!(
        transcript
            .iter()
            .any(|item| item.text.contains("История восстановлена."))
    );

    let turns: Vec<_> = projection
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::TurnSettled(settled)
                if settled.status == TurnSettlementStatus::Success =>
            {
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
