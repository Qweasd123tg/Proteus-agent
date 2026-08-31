use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use proteus_contracts::{
    contracts::{
        ApprovalPolicy, CancellationToken, ExecutionAttribution, ExecutionPermissionGrants,
        ExecutionScope, PolicyContext, PolicyVisibilityContext, SearchQuery, ToolExecutionRecorder,
    },
    domain::{
        MemoryItem, PermissionMode, PolicyDecision, ToolCall, ToolCallResolution, ToolResult,
    },
};
use proteus_core::{
    core::{
        AgentRuntime, AppConfig, BoundTools, HeadlessApprovalTransport, JournalEntry,
        ModelExecutionBinding, ModuleEpoch, PreparedAssembly, RuntimeSnapshot, SessionStore,
        ToolExecutionBinding,
    },
    process_adapters::ProcessComponentConfig,
};
use serde_json::json;
use tokio::time::{Duration, sleep, timeout};

fn workspace_file(path: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
        .display()
        .to_string()
}

#[test]
fn permission_grants_are_bound_to_one_execution_context() {
    let workspace = tempfile::tempdir().expect("workspace");
    let assembly =
        PreparedAssembly::from_config(AppConfig::default(), workspace.path().to_path_buf(), None)
            .expect("prepared assembly");
    let snapshot = RuntimeSnapshot::new(ModuleEpoch::initial(), assembly, None);
    let execution_a = snapshot.registry.execution_context(
        ModelExecutionBinding::detached(ExecutionScope::fresh(CancellationToken::new())),
        Arc::new(HeadlessApprovalTransport),
        PermissionMode::Normal,
    );
    let execution_b = snapshot.registry.execution_context(
        ModelExecutionBinding::detached(ExecutionScope::fresh(CancellationToken::new())),
        Arc::new(HeadlessApprovalTransport),
        PermissionMode::Normal,
    );

    execution_a
        .permission_grants
        .grant(["escalated_exec".to_owned()]);
    let child_binding = execution_a
        .clone()
        .with_fresh_permission_grants()
        .with_cancellation(execution_a.scope.cancellation.child_token());

    assert_eq!(
        execution_a.permission_grants.snapshot(),
        vec!["escalated_exec"]
    );
    assert!(execution_b.permission_grants.snapshot().is_empty());
    assert!(child_binding.permission_grants.snapshot().is_empty());
    assert_eq!(
        child_binding.scope.execution_id,
        execution_a.scope.execution_id
    );
    assert_ne!(
        execution_a.scope.execution_id,
        execution_b.scope.execution_id
    );
}

fn process_search_config() -> AppConfig {
    let component: ProcessComponentConfig = serde_json::from_value(json!({
        "command": "sh",
        "args": [
            workspace_file("crates/proteus-core/tests/fixtures/process_search.sh"),
            "static",
            "execution-boundary-search",
            "execution-boundary-component",
        ],
        "handshake_timeout_ms": 3_000,
        "exports": {
            "search": {"execution-boundary-search": {"timeout_ms": 3_000}},
        },
    }))
    .expect("valid process component");
    let mut config = AppConfig::default();
    config.modules.search = Some("execution-boundary-search".to_owned());
    config
        .components
        .insert("execution-boundary-component".to_owned(), component);
    config
}

#[tokio::test]
async fn process_search_runs_through_execution_context_without_chat_identity() {
    let workspace = tempfile::tempdir().expect("workspace");
    let assembly = PreparedAssembly::from_config(
        process_search_config(),
        workspace.path().to_path_buf(),
        None,
    )
    .expect("prepared assembly");
    let snapshot = RuntimeSnapshot::new(ModuleEpoch::initial(), assembly, None);
    let execution = snapshot.registry.execution_context(
        ModelExecutionBinding::detached(ExecutionScope::fresh(CancellationToken::new())),
        Arc::new(HeadlessApprovalTransport),
        PermissionMode::Normal,
    );

    let chunks = execution
        .search
        .search(SearchQuery::new(
            "needle",
            workspace.path().to_path_buf(),
            5,
        ))
        .await
        .expect("process-backed search through generic execution boundary");

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].source, "process:execution-boundary-search");
    assert_eq!(chunks[0].content, "hit from execution-boundary-search");
    assert!(!execution.is_cancelled());
}

#[test]
fn bound_tools_source_does_not_import_chat_domain_types() {
    for (path, source) in [
        (
            "bound_memory.rs",
            include_str!("../src/core/bound_memory.rs"),
        ),
        ("bound_tools.rs", include_str!("../src/core/bound_tools.rs")),
        (
            "bound_tools/support.rs",
            include_str!("../src/core/bound_tools/support.rs"),
        ),
    ] {
        for forbidden in [
            "AgentWorkflowContext",
            "SessionId",
            "ThreadId",
            "TurnId",
            "AgentTask",
            "AgentOutput",
            "CanonicalMessage",
        ] {
            assert!(
                !source.contains(forbidden),
                "generic execution binding source {path} imports chat-specific type {forbidden}"
            );
        }
    }
}

fn process_tool_config() -> AppConfig {
    let component: ProcessComponentConfig = serde_json::from_value(json!({
        "command": "sh",
        "args": [
            workspace_file("crates/proteus-core/tests/fixtures/process_tool.sh"),
            "execution-boundary-tools",
            "execution-boundary-tool-component",
        ],
        "handshake_timeout_ms": 3_000,
        "exports": {
            "tool": {"execution-boundary-tools": {"timeout_ms": 3_000}},
        },
    }))
    .expect("valid process tool component");
    let mut config = AppConfig::default();
    config.tools.enabled = vec!["detached_probe".to_owned()];
    config
        .components
        .insert("execution-boundary-tool-component".to_owned(), component);
    config
}

#[derive(Default)]
struct RecordingAllowPolicy {
    evaluations: AtomicUsize,
}

impl ApprovalPolicy for RecordingAllowPolicy {
    fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        self.evaluations.fetch_add(1, Ordering::SeqCst);
        PolicyDecision::Allow
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

#[derive(Default)]
struct RecordingToolFacts {
    attributions: Mutex<Vec<ExecutionAttribution>>,
}

impl RecordingToolFacts {
    fn push(&self, attribution: ExecutionAttribution) {
        self.attributions.lock().unwrap().push(attribution);
    }
}

#[async_trait]
impl ToolExecutionRecorder for RecordingToolFacts {
    async fn tool_call_requested(
        &self,
        attribution: ExecutionAttribution,
        _call: &ToolCall,
    ) -> anyhow::Result<()> {
        self.push(attribution);
        Ok(())
    }

    async fn tool_call_resolved(
        &self,
        attribution: ExecutionAttribution,
        _call: &ToolCall,
        _resolution: &ToolCallResolution,
    ) -> anyhow::Result<()> {
        self.push(attribution);
        Ok(())
    }

    async fn tool_approval_requested(
        &self,
        attribution: ExecutionAttribution,
        _call: &ToolCall,
        _reason: &str,
    ) -> anyhow::Result<()> {
        self.push(attribution);
        Ok(())
    }

    async fn tool_result_recorded(
        &self,
        attribution: ExecutionAttribution,
        _result: &ToolResult,
    ) -> anyhow::Result<()> {
        self.push(attribution);
        Ok(())
    }
}

#[tokio::test]
async fn bound_process_tool_executes_without_chat_or_agent_context() {
    let workspace = tempfile::tempdir().expect("workspace");
    let assembly =
        PreparedAssembly::from_config(process_tool_config(), workspace.path().to_path_buf(), None)
            .expect("prepared tool assembly");
    let snapshot = RuntimeSnapshot::new(ModuleEpoch::initial(), assembly, None);
    let scope = ExecutionScope::fresh(CancellationToken::new());
    let execution_id = scope.execution_id;
    let recorder = Arc::new(RecordingToolFacts::default());
    let policy = Arc::new(RecordingAllowPolicy::default());
    let binding = ToolExecutionBinding::detached(scope).with_recorder(recorder.clone());
    let tools = BoundTools::new(
        snapshot.registry.tools.clone(),
        policy.clone(),
        Arc::new(HeadlessApprovalTransport),
        Arc::<ExecutionPermissionGrants>::default(),
        binding,
    );

    assert_eq!(
        tools
            .visible_specs(workspace.path())
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>(),
        vec!["detached_probe"]
    );
    let result = tools
        .execute(
            workspace.path().to_path_buf(),
            ToolCall::new("detached-call", "detached_probe", json!({})),
        )
        .await
        .expect("detached process tool execution");

    assert!(result.ok, "{result:?}");
    assert_eq!(result.output, "detached process tool result");
    assert_eq!(result.metadata["saw_detached_attribution"], true);
    assert_eq!(policy.evaluations.load(Ordering::SeqCst), 1);
    let attributions = recorder.attributions.lock().unwrap();
    assert_eq!(attributions.len(), 3);
    assert!(attributions.iter().all(|attribution| {
        attribution.execution_id == execution_id && attribution.agent.is_none()
    }));
}

fn phase8_process_config() -> AppConfig {
    let component: ProcessComponentConfig = serde_json::from_value(json!({
        "command": "python3",
        "args": [
            "-B",
            workspace_file("crates/proteus-core/tests/fixtures/process_phase8_tool.py"),
        ],
        "handshake_timeout_ms": 3_000,
        "exports": {
            "tool": {"phase8-tools": {"timeout_ms": 3_000}},
            "policy": {"phase8-allow-all": {"timeout_ms": 3_000}},
        },
    }))
    .expect("valid Phase 8 component");
    let mut config = AppConfig::default();
    config.modules.policy = Some("phase8-allow-all".to_owned());
    config.tools.enabled = vec!["phase8_probe".to_owned()];
    config
        .components
        .insert("phase8-execution-component".to_owned(), component);
    config
}

async fn phase8_runtime(workspace: &Path, state_root: &Path) -> AgentRuntime {
    let config_path = state_root.join("configs/proteus.toml");
    AgentRuntime::builder(phase8_process_config(), workspace.to_path_buf())
        .with_config_path(Some(&config_path))
        .build_async()
        .await
        .expect("Phase 8 AgentRuntime")
}

#[tokio::test]
async fn agent_runtime_records_a_detached_process_tool_execution_without_turn_state() {
    let workspace = tempfile::tempdir().expect("workspace");
    let state = tempfile::tempdir().expect("state root");
    let runtime = phase8_runtime(workspace.path(), state.path()).await;

    let result = runtime
        .execute_tool(
            ToolCall::new(
                "phase8-recorded-call",
                "phase8_probe",
                json!({"label": "recorded"}),
            ),
            CancellationToken::new(),
        )
        .await
        .expect("top-level tool execution");

    assert!(result.ok, "{result:?}");
    assert_eq!(result.call_id, "phase8-recorded-call");
    assert_eq!(result.output, "phase8:recorded");
    assert_eq!(result.metadata["saw_detached_attribution"], true);
    assert_eq!(runtime.history_len().await, 0);

    let session_dir = runtime.session_dir().expect("persisted runtime session");
    let projection = SessionStore::open(session_dir.to_path_buf())
        .expect("open Phase 8 session")
        .load_projection()
        .expect("load Phase 8 journal projection");
    assert_eq!(projection.records.len(), 3);
    assert!(projection.records.iter().all(|record| {
        record.execution_id.is_some() && record.thread_id.is_none() && record.turn_id.is_none()
    }));
    let execution_id = projection.records[0].execution_id;
    assert!(
        projection
            .records
            .iter()
            .all(|record| record.execution_id == execution_id)
    );
    assert!(projection.records.iter().all(|record| matches!(
        record.entry,
        JournalEntry::ToolCallRecorded(_) | JournalEntry::ToolResultRecorded(_)
    )));
}

async fn wait_for_file(path: &Path) {
    timeout(Duration::from_secs(2), async {
        while !path.exists() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {}", path.display()));
}

fn phase8_memory_config(record_path: &Path) -> AppConfig {
    let component: ProcessComponentConfig = serde_json::from_value(json!({
        "command": "python3",
        "args": [
            "-B",
            workspace_file("crates/proteus-core/tests/fixtures/process_phase8_memory.py"),
        ],
        "handshake_timeout_ms": 3_000,
        "exports": {
            "memory": {
                "phase8-memory": {
                    "timeout_ms": 5_000,
                }
            },
        },
    }))
    .expect("valid Phase 8B memory component");
    let mut config = AppConfig::default();
    config.modules.memory = Some("phase8-memory".to_owned());
    config.tools.enabled.clear();
    config.module_config.insert(
        "memory".to_owned(),
        [(
            "phase8-memory".to_owned(),
            json!({"record_path": record_path}),
        )]
        .into_iter()
        .collect(),
    );
    config
        .components
        .insert("phase8-memory-component".to_owned(), component);
    config
}

async fn phase8_memory_runtime(
    workspace: &Path,
    state_root: &Path,
    record_path: &Path,
) -> AgentRuntime {
    AgentRuntime::builder(phase8_memory_config(record_path), workspace.to_path_buf())
        .with_config_path(Some(&state_root.join("configs/proteus.toml")))
        .build_async()
        .await
        .expect("Phase 8B memory runtime")
}

#[tokio::test]
async fn agent_runtime_remember_uses_detached_memory_v2_without_tool_or_turn_state() {
    let workspace = tempfile::tempdir().expect("workspace");
    let state = tempfile::tempdir().expect("state root");
    let record_path = state.path().join("memory-records.jsonl");
    let runtime = phase8_memory_runtime(workspace.path(), state.path(), &record_path).await;

    assert!(
        runtime.tool_entries().await.is_empty(),
        "remember_fact must be disabled"
    );
    runtime
        .remember(
            MemoryItem::new(
                "preference",
                "direct user memory",
                json!({"source": "slash"}),
            ),
            CancellationToken::new(),
        )
        .await
        .expect("top-level remember");

    let record: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(&record_path)
            .expect("memory record")
            .lines()
            .next()
            .expect("one memory record"),
    )
    .expect("memory record JSON");
    assert_eq!(record["method"], "remember");
    assert_eq!(record["item"]["content"], "direct user memory");
    assert!(record["attribution"]["execution_id"].is_string());
    assert!(record["attribution"]["agent"].is_null());
    assert_eq!(runtime.history_len().await, 0);

    assert!(
        !runtime
            .session_dir()
            .expect("configured session path")
            .exists(),
        "memory must not create a session journal"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_runtime_memory_cancel_settles_and_keeps_process_sibling_alive() {
    let workspace = tempfile::tempdir().expect("workspace");
    let state = tempfile::tempdir().expect("state root");
    let record_path = state.path().join("memory-records.jsonl");
    let runtime =
        Arc::new(phase8_memory_runtime(workspace.path(), state.path(), &record_path).await);
    let started = state.path().join("memory-started");
    let cancel_marker = state.path().join("memory-canceled");
    let cancellation = CancellationToken::new();

    let blocked = tokio::spawn({
        let runtime = Arc::clone(&runtime);
        let cancellation = cancellation.clone();
        let started = started.clone();
        let cancel_marker = cancel_marker.clone();
        async move {
            runtime
                .remember(
                    MemoryItem::new(
                        "fact",
                        "blocked",
                        json!({
                            "wait_for_cancel": true,
                            "start_marker": started,
                            "cancel_marker": cancel_marker,
                        }),
                    ),
                    cancellation,
                )
                .await
        }
    });
    wait_for_file(&started).await;

    runtime
        .remember(
            MemoryItem::new("fact", "sibling", json!({})),
            CancellationToken::new(),
        )
        .await
        .expect("concurrent sibling remember");
    cancellation.cancel();

    let error = timeout(Duration::from_secs(2), blocked)
        .await
        .expect("canceled memory settled")
        .expect("canceled memory task joined")
        .expect_err("blocked memory must be canceled");
    assert!(format!("{error:#}").contains("canceled"), "{error:#}");
    wait_for_file(&cancel_marker).await;
    let records = std::fs::read_to_string(&record_path).expect("memory records");
    assert!(records.contains("sibling"));
    assert!(!records.contains("\"content\": \"blocked\""));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_process_memory_keeps_old_config_across_runtime_reload() {
    let workspace = tempfile::tempdir().expect("workspace");
    let state = tempfile::tempdir().expect("state");
    let old_records = state.path().join("old-memory.jsonl");
    let new_records = state.path().join("new-memory.jsonl");
    let started = state.path().join("old-memory-started");
    let runtime = Arc::new(
        AgentRuntime::builder(
            phase8_memory_config(&old_records),
            workspace.path().to_path_buf(),
        )
        .build_async()
        .await
        .expect("old memory runtime"),
    );

    let old_call = tokio::spawn({
        let runtime = Arc::clone(&runtime);
        let started = started.clone();
        async move {
            runtime
                .remember(
                    MemoryItem::new(
                        "fact",
                        "old selected store",
                        json!({"start_marker": started, "delay_ms": 250}),
                    ),
                    CancellationToken::new(),
                )
                .await
        }
    });
    wait_for_file(&started).await;

    let next_config = phase8_memory_config(&new_records);
    let next = PreparedAssembly::from_config(next_config, workspace.path().to_path_buf(), None)
        .expect("new memory assembly");
    runtime
        .reload_assembly(next, None)
        .await
        .expect("reload memory assembly");

    old_call
        .await
        .expect("old remember joined")
        .expect("old remember completed");
    runtime
        .remember(
            MemoryItem::new("fact", "new selected store", json!({})),
            CancellationToken::new(),
        )
        .await
        .expect("new remember completed");

    assert!(
        std::fs::read_to_string(old_records)
            .expect("old records")
            .contains("old selected store")
    );
    assert!(
        std::fs::read_to_string(new_records)
            .expect("new records")
            .contains("new selected store")
    );
    assert_eq!(runtime.module_epoch().await.as_u64(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_runtime_forwards_targeted_cancel_without_serializing_a_process_sibling() {
    let workspace = tempfile::tempdir().expect("workspace");
    let state = tempfile::tempdir().expect("state root");
    let runtime = Arc::new(phase8_runtime(workspace.path(), state.path()).await);
    let canceled_started = state.path().join("canceled-started");
    let sibling_started = state.path().join("sibling-started");
    let cancel_marker = state.path().join("cancel-forwarded");
    let cancellation = CancellationToken::new();

    let canceled_task = tokio::spawn({
        let runtime = Arc::clone(&runtime);
        let cancellation = cancellation.clone();
        let canceled_started = canceled_started.clone();
        let cancel_marker = cancel_marker.clone();
        async move {
            runtime
                .execute_tool(
                    ToolCall::new(
                        "phase8-canceled-call",
                        "phase8_probe",
                        json!({
                            "wait_for_cancel": true,
                            "start_marker": canceled_started,
                            "cancel_marker": cancel_marker,
                        }),
                    ),
                    cancellation,
                )
                .await
        }
    });
    wait_for_file(&canceled_started).await;

    let sibling_task = tokio::spawn({
        let runtime = Arc::clone(&runtime);
        let sibling_started = sibling_started.clone();
        async move {
            runtime
                .execute_tool(
                    ToolCall::new(
                        "phase8-sibling-call",
                        "phase8_probe",
                        json!({
                            "label": "sibling",
                            "delay_ms": 150,
                            "start_marker": sibling_started,
                        }),
                    ),
                    CancellationToken::new(),
                )
                .await
        }
    });
    wait_for_file(&sibling_started).await;
    cancellation.cancel();

    let canceled = timeout(Duration::from_secs(2), canceled_task)
        .await
        .expect("canceled execution settled")
        .expect("canceled task joined")
        .expect("canceled execution result");
    let sibling = timeout(Duration::from_secs(2), sibling_task)
        .await
        .expect("sibling execution settled")
        .expect("sibling task joined")
        .expect("sibling execution result");

    assert!(!canceled.ok, "{canceled:?}");
    assert_eq!(canceled.call_id, "phase8-canceled-call");
    assert_eq!(canceled.metadata["canceled"], true);
    assert!(sibling.ok, "{sibling:?}");
    assert_eq!(sibling.call_id, "phase8-sibling-call");
    assert_eq!(sibling.output, "phase8:sibling");
    wait_for_file(&cancel_marker).await;

    let projection = SessionStore::open(
        runtime
            .session_dir()
            .expect("persisted runtime session")
            .to_path_buf(),
    )
    .expect("open concurrent Phase 8 session")
    .load_projection()
    .expect("load concurrent Phase 8 journal");
    let execution_for = |call_id: &str| {
        projection.records.iter().find_map(|record| {
            let matches_call = match &record.entry {
                JournalEntry::ToolCallRecorded(fact) => fact.call.id == call_id,
                JournalEntry::ToolResultRecorded(fact) => fact.result.call_id == call_id,
                _ => false,
            };
            matches_call.then_some(record.execution_id)
        })
    };
    let canceled_execution = execution_for("phase8-canceled-call").flatten();
    let sibling_execution = execution_for("phase8-sibling-call").flatten();
    assert!(canceled_execution.is_some());
    assert!(sibling_execution.is_some());
    assert_ne!(canceled_execution, sibling_execution);
    assert!(
        projection
            .records
            .iter()
            .all(|record| record.thread_id.is_none() && record.turn_id.is_none())
    );
}

#[tokio::test]
async fn agent_runtime_timeout_reaches_the_process_invocation() {
    let workspace = tempfile::tempdir().expect("workspace");
    let state = tempfile::tempdir().expect("state root");
    let runtime = phase8_runtime(workspace.path(), state.path()).await;
    let started = state.path().join("timeout-started");
    let cancel_marker = state.path().join("timeout-cancel-forwarded");

    let result = timeout(
        Duration::from_secs(2),
        runtime.execute_tool(
            ToolCall::new(
                "phase8-timeout-call",
                "phase8_probe",
                json!({
                    "wait_for_cancel": true,
                    "start_marker": started,
                    "cancel_marker": cancel_marker,
                }),
            ),
            CancellationToken::new(),
        ),
    )
    .await
    .expect("timed-out top-level call settled")
    .expect("timed-out top-level result");

    assert!(!result.ok, "{result:?}");
    assert_eq!(result.call_id, "phase8-timeout-call");
    assert_eq!(result.metadata["timed_out"], true);
    assert_eq!(result.metadata["timeout_ms"], 750);
    wait_for_file(&started).await;
    wait_for_file(&cancel_marker).await;
}

#[test]
fn phase8_runtime_source_exposes_no_ambient_execution_bag_or_chat_types() {
    let source = include_str!("../src/core/runtime/execution.rs");
    for forbidden in [
        "ExecutionContext",
        "RuntimeRegistry",
        "ToolRegistry",
        "AgentWorkflowContext",
        "SessionId",
        "ThreadId",
        "TurnId",
        "AgentTask",
        "AgentOutput",
        "CanonicalMessage",
    ] {
        assert!(
            !source.contains(forbidden),
            "Phase 8 top-level execution source exposes forbidden type {forbidden}"
        );
    }
}
