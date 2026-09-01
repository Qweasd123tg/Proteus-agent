use std::{borrow::Cow, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use futures_util::stream;
use tokio::sync::{Mutex, Semaphore};

use super::*;
use crate::{
    contracts::{
        ApprovalPolicy, EventSink, Model, ModelEventStream, PolicyContext, PolicyVisibilityContext,
        Workflow, WorkflowOutput,
    },
    core::{ConfiguredToolConfig, ConfiguredToolExecutorConfig, SessionConfigSnapshot},
    domain::{
        Event, EventEnvelope, ModelRef, PermissionMode, PolicyDecision, ReasoningConfig, ToolCall,
        ToolSafety, ToolSpec,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, FinishReason, MessageRole,
        ModelCapabilities, ModelStreamEvent,
    },
};

struct TurnStartedGate {
    block_once: AtomicBool,
    entered: Semaphore,
    proceed: Semaphore,
}

impl TurnStartedGate {
    fn new() -> Self {
        Self {
            block_once: AtomicBool::new(true),
            entered: Semaphore::new(0),
            proceed: Semaphore::new(0),
        }
    }
}

#[async_trait]
impl EventSink for TurnStartedGate {
    async fn append(&self, envelope: EventEnvelope) -> Result<()> {
        if matches!(envelope.event, Event::TurnStarted { .. })
            && self.block_once.swap(false, Ordering::SeqCst)
        {
            self.entered.add_permits(1);
            self.proceed
                .acquire()
                .await
                .expect("turn gate remains open")
                .forget();
        }
        Ok(())
    }
}

struct LabeledModel(&'static str);

struct AskPolicy;

impl ApprovalPolicy for AskPolicy {
    fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        PolicyDecision::Ask {
            reason: "snapshot policy probe".to_owned(),
        }
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Ask {
            reason: "snapshot policy probe".to_owned(),
        }
    }
}

#[async_trait]
impl Model for LabeledModel {
    fn id(&self) -> Cow<'static, str> {
        Cow::Borrowed(self.0)
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }

    async fn stream(&self, _request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let response = CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, self.0),
            Vec::new(),
            FinishReason::Stop,
        );
        Ok(Box::pin(stream::iter([Ok(ModelStreamEvent::Response {
            response,
        })])))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutionObservation {
    provider_id: String,
    provider_response: String,
    model_ref: ModelRef,
    reasoning: ReasoningConfig,
    write_decision: PolicyDecision,
    has_late_tool: bool,
}

struct AtomicityProbeWorkflow {
    observations: Arc<Mutex<Vec<ExecutionObservation>>>,
}

#[async_trait]
impl Workflow for AtomicityProbeWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: AgentWorkflowContext,
    ) -> Result<WorkflowOutput> {
        let model_ref = ctx.model_ref.clone();
        let reasoning = ctx.reasoning.clone();
        let provider_id = ctx.execution.model.id().into_owned();
        let response = ctx
            .execution
            .model
            .complete(
                CanonicalModelRequest::new(model_ref.clone(), history.clone())
                    .with_reasoning(reasoning.clone()),
            )
            .await?;
        let write_spec = ToolSpec::new(
            "write_probe",
            "snapshot policy probe",
            serde_json::json!({"type": "object"}),
            ToolSafety::WritesFiles,
        );
        let write_decision = ctx.execution.policy.evaluate(
            &ToolCall::new("snapshot-call", "write_probe", serde_json::json!({})),
            &PolicyContext::new(task.cwd.clone(), Some(write_spec)),
        );
        self.observations.lock().await.push(ExecutionObservation {
            provider_id,
            provider_response: message_text_for_test(&response.messages[0]),
            model_ref,
            reasoning,
            write_decision,
            has_late_tool: ctx.execution.tools.spec("late_tool").is_ok(),
        });
        Ok(successful_messages(history, task, "done"))
    }
}

#[tokio::test]
async fn admitted_turn_freezes_registry_and_effective_settings_until_settlement() {
    let config_root = tempfile::tempdir().expect("config root");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = config_root.path().join("configs").join("config.toml");
    let gate = Arc::new(TurnStartedGate::new());
    let observations = Arc::new(Mutex::new(Vec::new()));
    let workflow = Arc::new(AtomicityProbeWorkflow {
        observations: observations.clone(),
    });
    let runtime = Arc::new(
        AgentRuntime::builder(AppConfig::default(), workspace.path().to_path_buf())
            .with_config_path(Some(&config_path))
            .with_module_catalog(test_catalog())
            .with_event_sink(gate.clone())
            .build()
            .expect("runtime"),
    );
    {
        let mut state = runtime.services.execution_state.write().await;
        state.runtime.registry.workflow = workflow.clone();
        state.runtime.registry.policy = Arc::new(AskPolicy);
        state
            .runtime
            .registry
            .replace_model_for_test(Arc::new(LabeledModel("provider-a")));
    }
    let model_a = ModelRef::new("provider-a", "model-a");
    runtime.set_model_ref(model_a.clone()).await;
    runtime.set_reasoning_effort(Some("low".to_owned())).await;
    runtime.set_permission_mode(PermissionMode::Normal).await;

    let running_runtime = runtime.clone();
    let first = tokio::spawn(async move { running_runtime.run("first".to_owned()).await });
    gate.entered
        .acquire()
        .await
        .expect("first turn reaches post-admission gate")
        .forget();

    let mut next_config = AppConfig::default();
    next_config.tools.configured.push(ConfiguredToolConfig {
        name: "late_tool".to_owned(),
        description: "Appears in the next runtime epoch".to_owned(),
        input_schema: serde_json::json!({"type": "object"}),
        surface: crate::domain::ToolSurface::default(),
        safety: ToolSafety::ReadOnly,
        timeout_ms: None,
        metadata: serde_json::Value::Null,
        executor: ConfiguredToolExecutorConfig::Process {
            command: "printf".to_owned(),
            args: vec!["ok".to_owned()],
            environment: Default::default(),
        },
    });
    let mut next_assembly = PreparedAssembly::from_catalog(
        next_config.clone(),
        workspace.path().to_path_buf(),
        None,
        test_catalog(),
    )
    .expect("next assembly");
    next_assembly.registry_mut().workflow = workflow;
    next_assembly.registry_mut().policy = Arc::new(AskPolicy);
    next_assembly
        .registry_mut()
        .replace_model_for_test(Arc::new(LabeledModel("provider-b")));
    let next_config_snapshot = SessionConfigSnapshot::from_runtime_config(
        &next_config,
        next_assembly.registry(),
        PermissionMode::Normal,
    );
    runtime
        .reload_assembly(next_assembly, Some(next_config_snapshot))
        .await
        .expect("reload runtime");
    let model_b = ModelRef::new("provider-b", "model-b");
    runtime.set_model_ref(model_b.clone()).await;
    runtime.set_reasoning_effort(Some("high".to_owned())).await;
    runtime.set_permission_mode(PermissionMode::Plan).await;

    gate.proceed.add_permits(1);
    first
        .await
        .expect("first turn task")
        .expect("first turn output");
    runtime.run("second".to_owned()).await.expect("second turn");

    let observations = observations.lock().await.clone();
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0].provider_id, "provider-a");
    assert_eq!(observations[0].provider_response, "provider-a");
    assert_eq!(observations[0].model_ref, model_a);
    assert_eq!(observations[0].reasoning.effort.as_deref(), Some("low"));
    assert!(matches!(
        observations[0].write_decision,
        PolicyDecision::Ask { .. }
    ));
    assert!(!observations[0].has_late_tool);

    assert_eq!(observations[1].provider_id, "provider-b");
    assert_eq!(observations[1].provider_response, "provider-b");
    assert_eq!(observations[1].model_ref, model_b);
    assert_eq!(observations[1].reasoning.effort.as_deref(), Some("high"));
    assert!(matches!(
        observations[1].write_decision,
        PolicyDecision::Deny { .. }
    ));
    assert!(observations[1].has_late_tool);

    let opened = runtime
        .session
        .session_store
        .as_ref()
        .expect("session store")
        .load_records()
        .expect("journal records")
        .into_iter()
        .filter_map(|record| match record.entry {
            crate::core::JournalEntry::TurnOpened(opened) => Some(opened),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(opened.len(), 2);
    assert_eq!(opened[0].module_epoch, 0);
    assert_eq!(opened[1].module_epoch, 1);
    let recorded_a: SessionConfigSnapshot =
        serde_json::from_value(opened[0].config_snapshot.clone()).expect("config A");
    let recorded_b: SessionConfigSnapshot =
        serde_json::from_value(opened[1].config_snapshot.clone()).expect("config B");
    assert_eq!(recorded_a.model, observations[0].model_ref);
    assert_eq!(recorded_a.reasoning, observations[0].reasoning);
    assert_eq!(recorded_a.permission_mode_default, PermissionMode::Normal);
    assert_eq!(recorded_b.model, observations[1].model_ref);
    assert_eq!(recorded_b.reasoning, observations[1].reasoning);
    assert_eq!(recorded_b.permission_mode_default, PermissionMode::Plan);
}
