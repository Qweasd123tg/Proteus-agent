use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use tokio::{
    sync::Notify,
    time::{Duration, timeout},
};

use super::*;
use crate::{
    contracts::{
        AgentWorkflowContext, ApprovalPolicy, ApprovalRequest, ApprovalResponse, ApprovalTransport,
        CancellationToken, ExecutionAttribution, MemoryInvocationContext, MemoryStore,
        PolicyContext, PolicyVisibilityContext, Tool, ToolContext, ToolRegistry, ToolSource,
        Workflow, WorkflowOutput,
    },
    core::PreparedAssembly,
    domain::{
        AgentTask, MemoryItem, MemoryQuery, PolicyDecision, ToolCall, ToolResult, ToolSafety,
        ToolSpec,
    },
    model_standard::CanonicalMessage,
};

fn tool_spec() -> ToolSpec {
    tool_spec_with_safety(ToolSafety::ReadOnly)
}

fn tool_spec_with_safety(safety: ToolSafety) -> ToolSpec {
    ToolSpec::new(
        "phase8_unit_probe",
        "Phase 8 unit probe",
        serde_json::json!({"type": "object"}),
        safety,
    )
}

struct AskPolicy;

impl ApprovalPolicy for AskPolicy {
    fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        PolicyDecision::Ask {
            reason: "permission mode probe".to_owned(),
        }
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Ask {
            reason: "permission mode probe".to_owned(),
        }
    }
}

struct AllowPolicy;

impl ApprovalPolicy for AllowPolicy {
    fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        PolicyDecision::Allow
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

struct LabeledTool(&'static str);

#[async_trait]
impl Tool for LabeledTool {
    fn spec(&self) -> ToolSpec {
        tool_spec()
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> Result<ToolResult> {
        Ok(ToolResult::ok(call.id.clone(), self.0))
    }
}

struct BlockingLabeledTool {
    label: &'static str,
    started: Arc<Notify>,
    proceed: Arc<Notify>,
}

#[async_trait]
impl Tool for BlockingLabeledTool {
    fn spec(&self) -> ToolSpec {
        tool_spec()
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> Result<ToolResult> {
        self.started.notify_one();
        self.proceed.notified().await;
        Ok(ToolResult::ok(call.id.clone(), self.label))
    }
}

fn one_tool_registry(tool: Arc<dyn Tool>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register_arc(
            ToolSource::Dynamic {
                origin: "phase8-test".to_owned(),
            },
            tool,
        )
        .expect("register Phase 8 test tool");
    registry
}

#[derive(Default)]
struct RecordingMemory {
    items: tokio::sync::Mutex<Vec<MemoryItem>>,
    attributions: tokio::sync::Mutex<Vec<ExecutionAttribution>>,
}

#[async_trait]
impl MemoryStore for RecordingMemory {
    async fn remember(&self, item: MemoryItem, ctx: MemoryInvocationContext) -> Result<()> {
        self.items.lock().await.push(item);
        self.attributions.lock().await.push(ctx.attribution);
        Ok(())
    }

    async fn recall(
        &self,
        _query: MemoryQuery,
        _ctx: MemoryInvocationContext,
    ) -> Result<Vec<MemoryItem>> {
        Ok(self.items.lock().await.clone())
    }
}

struct BlockingMemory {
    started: Arc<Notify>,
    proceed: Arc<Notify>,
    items: tokio::sync::Mutex<Vec<MemoryItem>>,
}

#[async_trait]
impl MemoryStore for BlockingMemory {
    async fn remember(&self, item: MemoryItem, _ctx: MemoryInvocationContext) -> Result<()> {
        self.started.notify_one();
        self.proceed.notified().await;
        self.items.lock().await.push(item);
        Ok(())
    }

    async fn recall(
        &self,
        _query: MemoryQuery,
        _ctx: MemoryInvocationContext,
    ) -> Result<Vec<MemoryItem>> {
        Ok(self.items.lock().await.clone())
    }
}

#[tokio::test]
async fn top_level_memory_keeps_its_snapshot_across_reload() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let runtime = Arc::new(
        AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime"),
    );
    let started = Arc::new(Notify::new());
    let proceed = Arc::new(Notify::new());
    let old = Arc::new(BlockingMemory {
        started: started.clone(),
        proceed: proceed.clone(),
        items: tokio::sync::Mutex::new(Vec::new()),
    });
    runtime
        .services
        .execution_state
        .write()
        .await
        .runtime
        .registry
        .memory = old.clone();

    let running = tokio::spawn({
        let runtime = runtime.clone();
        async move {
            runtime
                .remember(
                    MemoryItem::new("fact", "old snapshot", serde_json::Value::Null),
                    CancellationToken::new(),
                )
                .await
        }
    });
    started.notified().await;

    let next = Arc::new(RecordingMemory::default());
    let mut next_assembly = PreparedAssembly::from_catalog(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        None,
        test_catalog(),
    )
    .expect("next assembly");
    next_assembly.registry_mut().memory = next.clone();
    runtime
        .reload_assembly(next_assembly, None)
        .await
        .expect("reload runtime");

    proceed.notify_one();
    running
        .await
        .expect("old task joined")
        .expect("old remember");
    runtime
        .remember(
            MemoryItem::new("fact", "new snapshot", serde_json::Value::Null),
            CancellationToken::new(),
        )
        .await
        .expect("new remember");

    assert_eq!(old.items.lock().await[0].content, "old snapshot");
    assert_eq!(next.items.lock().await[0].content, "new snapshot");
    assert_eq!(runtime.module_epoch().await.as_u64(), 1);
}

#[tokio::test]
async fn every_top_level_memory_operation_gets_fresh_detached_attribution() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let memory = Arc::new(RecordingMemory::default());
    let runtime = AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
        .with_module_catalog(test_catalog())
        .build()
        .expect("runtime");
    runtime
        .services
        .execution_state
        .write()
        .await
        .runtime
        .registry
        .memory = memory.clone();

    for content in ["first", "second"] {
        runtime
            .remember(
                MemoryItem::new("fact", content, serde_json::Value::Null),
                CancellationToken::new(),
            )
            .await
            .expect("remember");
    }

    let attributions = memory.attributions.lock().await;
    assert_eq!(attributions.len(), 2);
    assert!(attributions.iter().all(|value| value.agent.is_none()));
    assert_ne!(attributions[0].execution_id, attributions[1].execution_id);
}

#[tokio::test]
async fn pre_canceled_top_level_memory_never_reaches_the_store() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let memory = Arc::new(RecordingMemory::default());
    let runtime = AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
        .with_module_catalog(test_catalog())
        .build()
        .expect("runtime");
    runtime
        .services
        .execution_state
        .write()
        .await
        .runtime
        .registry
        .memory = memory.clone();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = runtime
        .remember(
            MemoryItem::new("fact", "must not write", serde_json::Value::Null),
            cancellation,
        )
        .await
        .expect_err("pre-canceled remember must fail");

    assert!(error.to_string().contains("canceled"));
    assert!(memory.items.lock().await.is_empty());
    assert!(memory.attributions.lock().await.is_empty());
}

#[tokio::test]
async fn top_level_tool_keeps_its_snapshot_across_reload() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let runtime = Arc::new(
        AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime"),
    );
    let started = Arc::new(Notify::new());
    let proceed = Arc::new(Notify::new());
    {
        let mut state = runtime.services.execution_state.write().await;
        state.runtime.registry.tools = one_tool_registry(Arc::new(BlockingLabeledTool {
            label: "old",
            started: started.clone(),
            proceed: proceed.clone(),
        }));
        state.runtime.registry.policy = Arc::new(AllowPolicy);
    }

    let running = tokio::spawn({
        let runtime = runtime.clone();
        async move {
            runtime
                .execute_tool(
                    ToolCall::new("old-call", "phase8_unit_probe", serde_json::json!({})),
                    CancellationToken::new(),
                )
                .await
        }
    });
    started.notified().await;

    let mut next_assembly = PreparedAssembly::from_catalog(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        None,
        test_catalog(),
    )
    .expect("next assembly");
    next_assembly.registry_mut().tools = one_tool_registry(Arc::new(LabeledTool("new")));
    next_assembly.registry_mut().policy = Arc::new(AllowPolicy);
    runtime
        .reload_assembly(next_assembly, None)
        .await
        .expect("reload runtime");

    proceed.notify_one();
    let old = running
        .await
        .expect("old task joined")
        .expect("old execution");
    let new = runtime
        .execute_tool(
            ToolCall::new("new-call", "phase8_unit_probe", serde_json::json!({})),
            CancellationToken::new(),
        )
        .await
        .expect("new execution");

    assert_eq!(old.output, "old");
    assert_eq!(new.output, "new");
    assert_eq!(runtime.module_epoch().await.as_u64(), 1);
}

struct BlockingWriteTool {
    started: Arc<Notify>,
    proceed: Arc<Notify>,
}

#[async_trait]
impl Tool for BlockingWriteTool {
    fn spec(&self) -> ToolSpec {
        tool_spec_with_safety(ToolSafety::WritesFiles)
    }

    async fn invoke(&self, call: &ToolCall, _ctx: ToolContext) -> Result<ToolResult> {
        self.started.notify_one();
        self.proceed.notified().await;
        Ok(ToolResult::ok(call.id.clone(), "frozen-auto"))
    }
}

#[tokio::test]
async fn admitted_top_level_tool_freezes_permission_mode() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let runtime = Arc::new(
        AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime"),
    );
    let started = Arc::new(Notify::new());
    let proceed = Arc::new(Notify::new());
    runtime.set_permission_mode(PermissionMode::Auto).await;
    {
        let mut state = runtime.services.execution_state.write().await;
        state.runtime.registry.tools = one_tool_registry(Arc::new(BlockingWriteTool {
            started: started.clone(),
            proceed: proceed.clone(),
        }));
        state.runtime.registry.policy = Arc::new(AskPolicy);
    }

    let running = tokio::spawn({
        let runtime = runtime.clone();
        async move {
            runtime
                .execute_tool(
                    ToolCall::new("auto-call", "phase8_unit_probe", serde_json::json!({})),
                    CancellationToken::new(),
                )
                .await
        }
    });
    started.notified().await;
    runtime.set_permission_mode(PermissionMode::Plan).await;
    proceed.notify_one();

    let admitted = running
        .await
        .expect("admitted task joined")
        .expect("admitted execution");
    assert!(admitted.ok, "{admitted:?}");
    assert_eq!(admitted.output, "frozen-auto");

    let next = runtime
        .execute_tool(
            ToolCall::new("plan-call", "phase8_unit_probe", serde_json::json!({})),
            CancellationToken::new(),
        )
        .await
        .expect("next execution");
    assert!(!next.ok, "{next:?}");
    assert!(next.text_or_status().contains("plan allows only read-only"));
}

struct BlockingWorkflow {
    started: Arc<Notify>,
    proceed: Arc<Notify>,
}

#[async_trait]
impl Workflow for BlockingWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        _ctx: AgentWorkflowContext,
    ) -> Result<WorkflowOutput> {
        self.started.notify_one();
        self.proceed.notified().await;
        Ok(successful_messages(history, task, "turn done"))
    }
}

#[tokio::test]
async fn top_level_tool_does_not_wait_for_the_turn_run_lock() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let runtime = Arc::new(
        AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime"),
    );
    let started = Arc::new(Notify::new());
    let proceed = Arc::new(Notify::new());
    {
        let mut state = runtime.services.execution_state.write().await;
        state.runtime.registry.tools = one_tool_registry(Arc::new(LabeledTool("detached")));
        state.runtime.registry.policy = Arc::new(AllowPolicy);
        state.runtime.registry.workflow = Arc::new(BlockingWorkflow {
            started: started.clone(),
            proceed: proceed.clone(),
        });
    }

    let turn = tokio::spawn({
        let runtime = runtime.clone();
        async move { runtime.run("hold turn".to_owned()).await }
    });
    started.notified().await;

    let detached = timeout(
        Duration::from_secs(1),
        runtime.execute_tool(
            ToolCall::new("parallel-call", "phase8_unit_probe", serde_json::json!({})),
            CancellationToken::new(),
        ),
    )
    .await
    .expect("top-level tool must not wait for Turn")
    .expect("top-level tool execution");
    assert_eq!(detached.output, "detached");

    proceed.notify_one();
    turn.await
        .expect("turn task joined")
        .expect("turn completed");
}

#[derive(Default)]
struct GrantsProbePolicy {
    observed: Mutex<Vec<Vec<String>>>,
}

impl ApprovalPolicy for GrantsProbePolicy {
    fn evaluate(&self, _call: &ToolCall, ctx: &PolicyContext) -> PolicyDecision {
        self.observed
            .lock()
            .expect("grant observations")
            .push(ctx.granted_permissions.clone());
        PolicyDecision::Ask {
            reason: "grant isolation probe".to_owned(),
        }
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

struct ApprovingTransport;

#[async_trait]
impl ApprovalTransport for ApprovingTransport {
    fn can_request_approval(&self) -> bool {
        true
    }

    async fn request_approval(&self, _request: ApprovalRequest) -> Result<ApprovalResponse> {
        Ok(ApprovalResponse::approve())
    }
}

struct GrantingTool {
    attributions: Arc<Mutex<Vec<ExecutionAttribution>>>,
}

#[async_trait]
impl Tool for GrantingTool {
    fn spec(&self) -> ToolSpec {
        tool_spec()
    }

    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult> {
        self.attributions
            .lock()
            .expect("attribution observations")
            .push(ctx.attribution);
        Ok(
            ToolResult::ok(call.id.clone(), "granted").with_metadata(serde_json::json!({
                "granted_permissions": ["phase8_permission"]
            })),
        )
    }
}

#[tokio::test]
async fn every_top_level_operation_gets_fresh_grants_and_attribution() {
    let cwd = tempfile::tempdir().expect("temp dir");
    let policy = Arc::new(GrantsProbePolicy::default());
    let attributions = Arc::new(Mutex::new(Vec::new()));
    let runtime = AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
        .with_module_catalog(test_catalog())
        .with_approval(Arc::new(ApprovingTransport))
        .build()
        .expect("runtime");
    {
        let mut state = runtime.services.execution_state.write().await;
        state.runtime.registry.tools = one_tool_registry(Arc::new(GrantingTool {
            attributions: attributions.clone(),
        }));
        state.runtime.registry.policy = policy.clone();
    }

    for call_id in ["grant-a", "grant-b"] {
        let result = runtime
            .execute_tool(
                ToolCall::new(call_id, "phase8_unit_probe", serde_json::json!({})),
                CancellationToken::new(),
            )
            .await
            .expect("grant probe execution");
        assert!(result.ok, "{result:?}");
    }

    assert_eq!(
        policy
            .observed
            .lock()
            .expect("grant observations")
            .as_slice(),
        &[Vec::<String>::new(), Vec::<String>::new()]
    );
    let attributions = attributions.lock().expect("attribution observations");
    assert_eq!(attributions.len(), 2);
    assert!(attributions.iter().all(|value| value.agent.is_none()));
    assert_ne!(attributions[0].execution_id, attributions[1].execution_id);
}
