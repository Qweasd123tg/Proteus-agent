use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use proteus_core::{
    contracts::{
        ApprovalPolicy, CancellationToken, ExecutionAttribution, ExecutionPermissionGrants,
        ExecutionScope, PolicyContext, PolicyVisibilityContext, SearchQuery, ToolExecutionRecorder,
    },
    core::{
        AppConfig, BoundTools, HeadlessApprovalTransport, ModuleEpoch, PreparedAssembly,
        RuntimeSnapshot, ToolExecutionBinding,
    },
    domain::{PermissionMode, PolicyDecision, ToolCall, ToolCallResolution, ToolResult},
    process_adapters::ProcessComponentConfig,
};
use serde_json::json;

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
        proteus_core::core::ModelExecutionBinding::detached(ExecutionScope::fresh(
            CancellationToken::new(),
        )),
        Arc::new(HeadlessApprovalTransport),
        PermissionMode::Normal,
    );
    let execution_b = snapshot.registry.execution_context(
        proteus_core::core::ModelExecutionBinding::detached(ExecutionScope::fresh(
            CancellationToken::new(),
        )),
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
        proteus_core::core::ModelExecutionBinding::detached(ExecutionScope::fresh(
            CancellationToken::new(),
        )),
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
                "generic BoundTools source {path} imports chat-specific type {forbidden}"
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
