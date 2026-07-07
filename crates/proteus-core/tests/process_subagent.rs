//! Round-trip тест process-subagent-а: реальный дочерний процесс
//! `proteus server stdio` (бинарь из CARGO_BIN_EXE_proteus) с минимальным
//! конфигом (fake model, workflow "none"), запуск роли, resume по task_id
//! и сброс истории между свежими задачами.

use std::{path::PathBuf, sync::Arc};

use proteus_core::{
    contracts::{
        ApprovalPolicy, EventEmitter, PolicyContext, PolicyVisibilityContext, SubagentRequest,
        SubagentRunner, SubagentStatus, ToolRegistry,
    },
    core::{
        HeadlessApprovalTransport, HeadlessUserInputTransport, InMemoryEventStore,
        ProcessSubagentRunner,
    },
    domain::{
        AgentTask, Event, ModelRef, PolicyDecision, ReasoningConfig, ToolCall, new_session_id,
        new_thread_id, new_turn_id,
    },
    stubs::{
        AllVisibleToolExposure, EmptyContextBuilder, FakeModelClient, NoCompactor, NoMemory,
        NoSubagent, NullPatchApplier, NullSearch,
    },
};
use serde_json::json;

struct AllowAllPolicy;

impl ApprovalPolicy for AllowAllPolicy {
    fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        PolicyDecision::Allow
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

fn test_runtime_context(
    events: Arc<InMemoryEventStore>,
) -> proteus_core::contracts::RuntimeContext {
    proteus_core::contracts::RuntimeContext::new(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        ModelRef::new("fake", "fake-tool-model"),
        ReasoningConfig::default(),
        120_000,
        30_000,
        Arc::new(EventEmitter::new(events)),
        Arc::new(FakeModelClient::default()),
        Arc::new(NullSearch),
        Arc::new(NoMemory),
        Arc::new(EmptyContextBuilder),
        ToolRegistry::new(),
        Arc::new(AllowAllPolicy),
        Arc::new(HeadlessApprovalTransport),
        Arc::new(HeadlessUserInputTransport),
        Arc::new(NullPatchApplier),
        Arc::new(NoCompactor),
        Arc::new(AllVisibleToolExposure),
        Arc::new(NoSubagent),
    )
}

/// Конфиг ребёнка: fake model (без сети), workflow "none" (мгновенный
/// детерминированный ответ), event log внутри tempdir.
fn write_child_config(config_home: &std::path::Path) -> PathBuf {
    let configs_dir = config_home.join("configs");
    std::fs::create_dir_all(&configs_dir).expect("create configs dir");
    let config_path = configs_dir.join("config.toml");
    std::fs::write(&config_path, "[modules]\nworkflow = \"none\"\n").expect("write child config");
    config_path
}

fn process_runner(config_path: &std::path::Path) -> ProcessSubagentRunner {
    ProcessSubagentRunner::from_config(json!({
        "binary": env!("CARGO_BIN_EXE_proteus"),
        "roles": [
            {
                "name": "helper",
                "description": "Stub helper child",
                "config": config_path.to_string_lossy(),
                "timeout_ms": 60000
            }
        ]
    }))
    .expect("build process runner")
}

#[tokio::test]
async fn process_subagent_round_trips_turn_resume_and_fresh_task() {
    let config_home = tempfile::tempdir().expect("config home");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = write_child_config(config_home.path());
    let runner = process_runner(&config_path);

    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events.clone());
    let parent_thread_id = ctx.thread_id;
    let task = AgentTask::new("delegate", workspace.path().to_path_buf());

    // Первый прогон: полный round-trip через дочерний процесс.
    let first = runner
        .run(
            SubagentRequest::new("helper", "first task", task.clone())
                .with_description("first delegation"),
            ctx.clone(),
        )
        .await
        .expect("first child turn");

    assert_eq!(first.status, SubagentStatus::Completed);
    assert!(
        first.summary.contains("workflow is disabled"),
        "summary: {}",
        first.summary
    );
    assert_eq!(first.metadata["resumable"], json!(true));
    let child_thread_id = first.child_thread_id.expect("child thread id");
    assert_ne!(child_thread_id, parent_thread_id);

    let envelopes = events.envelopes().await;
    let started = envelopes
        .iter()
        .find(|envelope| matches!(envelope.event, Event::SubagentStarted { .. }))
        .expect("SubagentStarted");
    assert_eq!(started.thread_id, parent_thread_id);
    let finished = envelopes
        .iter()
        .find_map(|envelope| match &envelope.event {
            Event::SubagentFinished { status, .. } => Some(status.clone()),
            _ => None,
        })
        .expect("SubagentFinished");
    assert_eq!(finished, "completed");

    // Resume по task_id: тот же процесс, тот же child_thread_id.
    let resumed = runner
        .run(
            SubagentRequest::new("helper", "continue the task", task.clone())
                .with_metadata(json!({ "task_id": child_thread_id.to_string() })),
            ctx.clone(),
        )
        .await
        .expect("resumed child turn");
    assert_eq!(resumed.status, SubagentStatus::Completed);
    assert_eq!(resumed.child_thread_id, Some(child_thread_id));

    // Свежая задача (без task_id): та же роль, но новый child thread —
    // история ребёнка сбрасывается через ClearHistory.
    let fresh = runner
        .run(SubagentRequest::new("helper", "brand new task", task), ctx)
        .await
        .expect("fresh child turn");
    assert_eq!(fresh.status, SubagentStatus::Completed);
    assert_ne!(fresh.child_thread_id, Some(child_thread_id));
}

#[tokio::test]
async fn process_subagent_rejects_unknown_task_id_and_foreign_role() {
    let config_home = tempfile::tempdir().expect("config home");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = write_child_config(config_home.path());
    let runner = process_runner(&config_path);

    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events);
    let task = AgentTask::new("delegate", workspace.path().to_path_buf());

    let unknown = runner
        .run(
            SubagentRequest::new("helper", "resume nothing", task.clone())
                .with_metadata(json!({ "task_id": new_thread_id().to_string() })),
            ctx.clone(),
        )
        .await
        .expect_err("unknown task_id must fail");
    assert!(
        unknown.to_string().contains("unknown task_id"),
        "{unknown:#}"
    );

    let missing_role = runner
        .run(SubagentRequest::new("mystery", "who", task), ctx)
        .await
        .expect_err("unknown role must fail");
    assert!(
        missing_role
            .to_string()
            .contains("unknown subagent role: mystery"),
        "{missing_role:#}"
    );
}
