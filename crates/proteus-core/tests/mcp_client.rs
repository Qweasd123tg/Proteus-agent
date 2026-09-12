use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use proteus_contracts::{
    contracts::{CancellationToken, ExecutionAttribution, Tool, ToolContext},
    domain::{ToolCall, ToolSafety, new_call_id, new_execution_id},
};
use proteus_core::core::{
    AppConfig, ConfiguredMcpServerConfig, ModuleCatalog, ProcessEnvironmentConfig,
};
use serde_json::{Value, json};

#[path = "support/model.rs"]
mod test_model;

struct Fixture {
    dir: tempfile::TempDir,
    config: AppConfig,
    events: PathBuf,
    generations: PathBuf,
}

impl Fixture {
    fn new(timeout_ms: u64, max_response_bytes: Option<usize>) -> Self {
        Self::with_init_mode(timeout_ms, max_response_bytes, "ok")
    }

    fn with_init_mode(timeout_ms: u64, max_response_bytes: Option<usize>, init_mode: &str) -> Self {
        let dir = tempfile::tempdir().expect("fixture temp dir");
        let events = dir.path().join("events.log");
        let generations = dir.path().join("generations.log");
        let mut config = AppConfig::default();
        config.tools.enabled.clear();
        config.agent_control.surface = proteus_core::core::AgentControlSurface::None;
        config.tools.mcp_servers.push(ConfiguredMcpServerConfig {
            name: "fixture".into(),
            command: "python3".into(),
            args: vec![server_path().display().to_string()],
            environment: ProcessEnvironmentConfig {
                env_allowlist: vec!["PATH".into()],
                env: BTreeMap::from([
                    ("MCP_FIXTURE_EVENT_LOG".into(), events.display().to_string()),
                    (
                        "MCP_FIXTURE_GENERATION_FILE".into(),
                        generations.display().to_string(),
                    ),
                    ("MCP_FIXTURE_INIT_MODE".into(), init_mode.into()),
                    ("MCP_LITERAL_ENV".into(), "literal-value".into()),
                ]),
            },
            protocol_version: "2025-11-25".into(),
            safety: ToolSafety::ReadOnly,
            supports_parallel_tool_calls: true,
            timeout_ms: Some(timeout_ms),
            max_response_bytes,
            metadata: Value::Null,
        });
        Self {
            dir,
            config,
            events,
            generations,
        }
    }

    fn registry(&self) -> anyhow::Result<proteus_contracts::contracts::ToolRegistry> {
        ModuleCatalog::from_config(&self.config)?
            .build_tools_for_inspection(&self.config, self.dir.path())
    }

    fn tool(
        &self,
        registry: &proteus_contracts::contracts::ToolRegistry,
        name: &str,
    ) -> Arc<dyn Tool> {
        registry
            .get(&format!("fixture__{name}"))
            .unwrap_or_else(|| panic!("missing discovered fixture tool {name}"))
    }

    fn lines(&self, path: &Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }

    fn event_lines(&self) -> Vec<String> {
        self.lines(&self.events)
    }

    fn generation_count(&self) -> usize {
        self.lines(&self.generations).len()
    }
}

fn server_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mcp_client/server.py")
}

fn context(cancellation: CancellationToken, cwd: &Path) -> ToolContext {
    let mut context = ToolContext::new(
        cwd.to_path_buf(),
        ExecutionAttribution::detached(new_execution_id()),
    );
    context.cancellation = cancellation;
    context
}

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall::new(new_call_id(), format!("fixture__{name}"), args)
}

#[tokio::test]
async fn discovery_paginates_and_persistent_calls_preserve_content_error_and_environment() {
    let fixture = Fixture::new(2_000, None);
    let registry = fixture.registry().expect("discover fixture MCP tools");
    assert_eq!(
        registry
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>(),
        vec![
            "fixture__delay_write",
            "fixture__echo",
            "fixture__fail",
            "fixture__oversize",
        ]
    );

    let echo = fixture.tool(&registry, "echo");
    let result = echo
        .invoke(
            &call("echo", json!({"label":"Привет"})),
            context(CancellationToken::new(), fixture.dir.path()),
        )
        .await
        .expect("echo call");
    assert!(result.ok);
    assert_eq!(result.output, "echo:Привет:generation:1");
    assert_eq!(result.metadata["structured_content"]["label"], "Привет");
    assert_eq!(result.metadata["structured_content"]["generation"], 1);
    assert_eq!(result.metadata["structured_content"]["path_present"], true);
    assert_eq!(result.metadata["structured_content"]["home_present"], false);
    assert_eq!(
        result.metadata["structured_content"]["literal_env"],
        "literal-value"
    );

    let rpc_error = echo
        .invoke(
            &call("echo", json!({"mode":"rpc_error"})),
            context(CancellationToken::new(), fixture.dir.path()),
        )
        .await
        .expect_err("ordinary JSON-RPC error remains a call error");
    assert!(rpc_error.to_string().contains("ordinary fixture RPC error"));
    let after_rpc_error = echo
        .invoke(
            &call("echo", json!({"label":"after-rpc-error"})),
            context(CancellationToken::new(), fixture.dir.path()),
        )
        .await
        .expect("ordinary RPC error must preserve the connection");
    assert_eq!(after_rpc_error.output, "echo:after-rpc-error:generation:1");

    let failure = fixture
        .tool(&registry, "fail")
        .invoke(
            &call("fail", json!({})),
            context(CancellationToken::new(), fixture.dir.path()),
        )
        .await
        .expect("isError remains a canonical tool result");
    assert!(!failure.ok);
    assert_eq!(failure.output, "fixture failure");
    assert_eq!(failure.error.as_deref(), Some("fixture failure"));
    assert_eq!(failure.metadata["structured_content"]["kind"], "expected");
    assert_eq!(
        fixture.generation_count(),
        1,
        "calls reuse discovered server"
    );

    let events = fixture.event_lines();
    assert_eq!(
        &events[..4],
        &["1:spawn", "1:initialize", "1:initialized", "1:list:first"]
    );
    assert!(events.iter().any(|line| line == "1:list:page-2"));
}

#[tokio::test]
async fn concurrent_calls_are_correlated_to_their_own_responses() {
    let fixture = Fixture::new(2_000, None);
    let registry = fixture.registry().expect("discover fixture MCP tools");
    let tool = fixture.tool(&registry, "echo");
    let slow_call = call("echo", json!({"label":"slow","delay_ms":150}));
    let fast_call = call("echo", json!({"label":"fast","delay_ms":10}));
    let slow = tool.invoke(
        &slow_call,
        context(CancellationToken::new(), fixture.dir.path()),
    );
    let fast = tool.invoke(
        &fast_call,
        context(CancellationToken::new(), fixture.dir.path()),
    );
    let (slow, fast) = tokio::join!(slow, fast);
    assert_eq!(
        slow.expect("slow response").output,
        "echo:slow:generation:1"
    );
    assert_eq!(
        fast.expect("fast response").output,
        "echo:fast:generation:1"
    );
    assert_eq!(fixture.generation_count(), 1);
}

#[tokio::test]
async fn cancellation_prevents_late_side_effect_and_next_use_starts_a_new_generation() {
    let fixture = Fixture::new(5_000, None);
    let registry = fixture.registry().expect("discover fixture MCP tools");
    let delayed = fixture.tool(&registry, "delay_write");
    let marker = fixture.dir.path().join("cancelled-side-effect");
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    let delayed_call = call("delay_write", json!({"marker":marker,"delay_ms":800}));
    let invoke = delayed.invoke(&delayed_call, context(cancellation, fixture.dir.path()));
    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
    });
    let error = invoke.await.expect_err("cancelled MCP call must fail");
    cancel_task.await.expect("cancel task");
    assert!(
        error.to_string().to_lowercase().contains("cancel"),
        "{error:#}"
    );
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(
        !marker.exists(),
        "cancelled call performed a late side effect"
    );

    let restarted = fixture
        .tool(&registry, "echo")
        .invoke(
            &call("echo", json!({"label":"restart"})),
            context(CancellationToken::new(), fixture.dir.path()),
        )
        .await
        .expect("next explicit invocation restarts MCP generation");
    assert_eq!(restarted.output, "echo:restart:generation:2");
    assert_eq!(fixture.generation_count(), 2);
    assert!(
        fixture
            .event_lines()
            .iter()
            .any(|line| line.starts_with("1:cancel_notification:"))
    );
}

#[tokio::test]
async fn dropping_an_in_flight_call_stops_its_generation_without_a_late_side_effect() {
    let fixture = Fixture::new(5_000, None);
    let registry = fixture.registry().expect("discover fixture MCP tools");
    let delayed = fixture.tool(&registry, "delay_write");
    let marker = fixture.dir.path().join("dropped-side-effect");
    let cwd = fixture.dir.path().to_path_buf();
    let task = tokio::spawn(async move {
        delayed
            .invoke(
                &call("delay_write", json!({"marker":marker,"delay_ms":800})),
                context(CancellationToken::new(), &cwd),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(
        !fixture.dir.path().join("dropped-side-effect").exists(),
        "dropped call performed a late side effect"
    );

    let restarted = fixture
        .tool(&registry, "echo")
        .invoke(
            &call("echo", json!({"label":"after-drop"})),
            context(CancellationToken::new(), fixture.dir.path()),
        )
        .await
        .expect("drop invalidates the old generation");
    assert_eq!(restarted.output, "echo:after-drop:generation:2");
}

#[tokio::test]
async fn timeout_and_bounded_response_invalidate_the_generation_without_replay() {
    let fixture = Fixture::new(120, Some(4_096));
    let registry = fixture.registry().expect("discover fixture MCP tools");
    let marker = fixture.dir.path().join("timed-out-side-effect");
    let error = fixture
        .tool(&registry, "delay_write")
        .invoke(
            &call("delay_write", json!({"marker":marker,"delay_ms":700})),
            context(CancellationToken::new(), fixture.dir.path()),
        )
        .await
        .expect_err("MCP call must honor configured timeout");
    assert!(error.to_string().contains("120ms"), "{error:#}");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        !marker.exists(),
        "timed-out call was replayed or completed late"
    );

    let oversized = fixture
        .tool(&registry, "oversize")
        .invoke(
            &call("oversize", json!({})),
            context(CancellationToken::new(), fixture.dir.path()),
        )
        .await
        .expect_err("oversized MCP response must be rejected");
    assert!(
        oversized.to_string().to_lowercase().contains("limit")
            || oversized.to_string().to_lowercase().contains("large")
            || oversized.to_string().to_lowercase().contains("exceeded"),
        "{oversized:#}"
    );

    let recovered = fixture
        .tool(&registry, "echo")
        .invoke(
            &call("echo", json!({"label":"after-errors"})),
            context(CancellationToken::new(), fixture.dir.path()),
        )
        .await
        .expect("next explicit call restarts after transport errors");
    assert!(recovered.output.contains("after-errors:generation:"));
    assert!(fixture.generation_count() >= 3);
    assert_eq!(
        fixture
            .event_lines()
            .iter()
            .filter(|line| line.contains(":call_started:") && line.ends_with(":delay_write"))
            .count(),
        1,
        "timed-out mutating call must not be replayed"
    );
}

#[test]
fn malformed_or_unsupported_initialize_is_rejected_during_discovery() {
    for mode in ["malformed", "wrong_version"] {
        let fixture = Fixture::with_init_mode(1_000, None, mode);
        let error = match fixture.registry() {
            Ok(_) => panic!("invalid initialize result must reject MCP server"),
            Err(error) => error,
        };
        let rendered = format!("{error:#}").to_lowercase();
        assert!(
            rendered.contains("initialize")
                || rendered.contains("protocol")
                || rendered.contains("deserialize"),
            "{mode}: {error:#}"
        );
    }
}

#[tokio::test]
async fn inline_mcp_tool_uses_runtime_approval_journal_and_replay_path() {
    use proteus_contracts::contracts::ApprovalCacheScope;
    use proteus_core::{
        app_server::AgentAppServer,
        core::{JournalEntry, replay_workflow},
    };

    let fixture = Fixture::new(2_000, None);
    let mut config = test_model::config();
    config.components.insert(
        "runtime".into(),
        serde_json::from_value(json!({
            "command": test_model::worker(),
            "exports": {
                "workflow":{"coding.single_loop":{}},
                "context":{"simple":{}},
                "policy":{"ask_write":{}}
            }
        }))
        .unwrap(),
    );
    config.modules.workflow = Some("coding.single_loop".into());
    config.modules.context = Some("simple".into());
    config.modules.policy = Some("ask_write".into());
    config.agent_control.surface = proteus_core::core::AgentControlSurface::None;
    config.tools.enabled.clear();
    config.tools.configured.push(
        serde_json::from_value(json!({
            "name": "read_file",
            "description": "read a file through the MCP fixture",
            "input_schema": {
                "type":"object",
                "properties":{"path":{"type":"string"}},
                "required":["path"],
                "additionalProperties":false
            },
            "safety": "RunsCommands",
            "timeout_ms": 2000,
            "executor": {
                "kind":"mcp",
                "server":"fixture-inline",
                "command":"python3",
                "args":[server_path()],
                "env_allowlist":["PATH"],
                "env": {
                    "MCP_FIXTURE_EVENT_LOG": fixture.events,
                    "MCP_FIXTURE_GENERATION_FILE": fixture.generations,
                    "MCP_LITERAL_ENV":"runtime"
                },
                "tool":"echo",
                "protocol_version":"2025-11-25"
            }
        }))
        .expect("inline MCP config"),
    );
    config.event_log.path = fixture.dir.path().join("runtime-events.jsonl");
    config.app_server.approval_timeout_ms = 0;
    let config_path = fixture.dir.path().join("runtime-config.json");
    std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let server = AgentAppServer::launch(
        config.clone(),
        fixture.dir.path().to_path_buf(),
        Some(&config_path),
    )
    .await
    .expect("launch runtime");
    server.start_session().await.expect("start session");
    let sender = server.clone();
    let turn =
        tokio::spawn(async move { sender.send_user_message("read_file probe.txt".into()).await });
    let approval = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(request) = server.pending_requests().await.approvals.into_iter().next() {
                break request;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("MCP approval request");
    assert_eq!(approval.call.name, "read_file");
    server
        .respond_approval(
            approval.approval_id.as_ref(),
            true,
            None,
            ApprovalCacheScope::None,
        )
        .await
        .expect("approve MCP call");
    let output = turn.await.expect("turn task").expect("runtime MCP turn");
    assert!(output.text.contains("generation:1"), "{}", output.text);

    let store = proteus_core::core::SessionStore::open(
        server
            .session_dir_path()
            .expect("runtime session directory"),
    )
    .unwrap();
    let records = store.load_records().unwrap();
    assert!(records.iter().any(|record| matches!(
        &record.entry,
        JournalEntry::ToolCallRecorded(call) if call.call.name == "read_file"
    )));
    assert!(records.iter().any(|record| matches!(
        &record.entry,
        JournalEntry::ToolResultRecorded(result) if result.result.ok
    )));
    server.shutdown().await;

    let replay = replay_workflow(
        store.journal_path(),
        &config,
        &ModuleCatalog::from_config(&config).unwrap(),
        Default::default(),
    )
    .await
    .expect("workflow replay");
    assert!(replay.comparison.matched, "{:?}", replay.comparison);
    assert!(replay.source_journal_unchanged);
}
