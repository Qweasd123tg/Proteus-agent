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

/// Параллельный запуск двух детей одной parallel_safe-роли через
/// spawn/wait: оба turn-а завершаются, дети живут на разных child thread.
/// Resume по task_id второго ребёнка продолжает его process-session
/// (resumable-учёт по process id, а не по слоту роли).
#[tokio::test]
async fn process_subagent_spawns_parallel_children_and_resumes_by_task_id() {
    let config_home = tempfile::tempdir().expect("config home");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = write_child_config(config_home.path());
    let runner = ProcessSubagentRunner::from_config(json!({
        "binary": env!("CARGO_BIN_EXE_proteus"),
        "roles": [
            {
                "name": "helper",
                "description": "Stub helper child",
                "config": config_path.to_string_lossy(),
                "parallel_safe": true,
                "max_processes": 2,
                "timeout_ms": 60000
            }
        ]
    }))
    .expect("build process runner");

    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events.clone());
    let task = AgentTask::new("delegate", workspace.path().to_path_buf());

    assert!(
        runner.roles()[0].parallel_safe,
        "role spec must carry parallel_safe"
    );

    let first = runner
        .spawn(
            SubagentRequest::new("helper", "branch one", task.clone()),
            ctx.clone(),
        )
        .await
        .expect("spawn first child");
    let second = runner
        .spawn(
            SubagentRequest::new("helper", "branch two", task.clone()),
            ctx.clone(),
        )
        .await
        .expect("spawn second child");
    assert_ne!(first.spawn_id, second.spawn_id);
    assert_ne!(first.child_thread_id, second.child_thread_id);

    // Оба Started уже в event stream — до первого wait.
    let started = events
        .envelopes()
        .await
        .iter()
        .filter(|envelope| matches!(envelope.event, Event::SubagentStarted { .. }))
        .count();
    assert_eq!(started, 2);

    let first_result = runner.wait(&first).await.expect("first child result");
    let second_result = runner.wait(&second).await.expect("second child result");
    assert_eq!(first_result.status, SubagentStatus::Completed);
    assert_eq!(second_result.status, SubagentStatus::Completed);

    // Resume второго ребёнка: его процесс в пуле, session-история жива.
    // (Первый resume-ить нельзя без гонки: если дети успели пройти через
    // один процесс, ClearHistory свежей задачи хоронит старые task_id-ы.)
    let resumed = runner
        .run(
            SubagentRequest::new("helper", "continue branch two", task)
                .with_metadata(json!({ "task_id": second.child_thread_id.to_string() })),
            ctx,
        )
        .await
        .expect("resume second child");
    assert_eq!(resumed.status, SubagentStatus::Completed);
    assert_eq!(resumed.child_thread_id, Some(second.child_thread_id));
}

/// История, очищенная под свежую задачу, хоронит старые task_id-ы того же
/// процесса: resume по ним честно отклоняется, а не продолжает пустую
/// session.
#[tokio::test]
async fn process_subagent_fresh_task_invalidates_prior_task_ids_of_reused_process() {
    let config_home = tempfile::tempdir().expect("config home");
    let workspace = tempfile::tempdir().expect("workspace");
    let config_path = write_child_config(config_home.path());
    let runner = process_runner(&config_path);

    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events);
    let task = AgentTask::new("delegate", workspace.path().to_path_buf());

    let first = runner
        .run(
            SubagentRequest::new("helper", "first task", task.clone()),
            ctx.clone(),
        )
        .await
        .expect("first child turn");
    let first_task_id = first.child_thread_id.expect("child thread id");

    // Свежая задача на той же роли (max_processes = 1 → тот же процесс):
    // ClearHistory стирает session-историю первого task-а.
    runner
        .run(
            SubagentRequest::new("helper", "fresh task", task.clone()),
            ctx.clone(),
        )
        .await
        .expect("fresh child turn");

    let stale = runner
        .run(
            SubagentRequest::new("helper", "continue first", task)
                .with_metadata(json!({ "task_id": first_task_id.to_string() })),
            ctx,
        )
        .await
        .expect_err("stale task_id must be rejected after history clear");
    assert!(stale.to_string().contains("unknown task_id"), "{stale:#}");
}

/// Fresh-задача с другим cwd не реюзает idle-процесс (его `--cwd`
/// зафиксирован при спавне): роль получает новый процесс, а session-история
/// первого не очищается — его task_id остаётся resumable. Без cwd-проверки
/// реюз ломал бы worktree-изоляцию пишущих детей.
#[tokio::test]
async fn process_subagent_fresh_task_with_different_cwd_spawns_new_process() {
    let config_home = tempfile::tempdir().expect("config home");
    let workspace_a = tempfile::tempdir().expect("workspace a");
    let workspace_b = tempfile::tempdir().expect("workspace b");
    let config_path = write_child_config(config_home.path());
    let runner = process_runner(&config_path);

    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events);

    let task_a = AgentTask::new("delegate", workspace_a.path().to_path_buf());
    let first = runner
        .run(
            SubagentRequest::new("helper", "first task", task_a.clone()),
            ctx.clone(),
        )
        .await
        .expect("first child turn");
    let first_task_id = first.child_thread_id.expect("child thread id");

    // Fresh-задача в другом cwd: должен подняться новый процесс, а не
    // ClearHistory на процессе из workspace_a.
    let task_b = AgentTask::new("delegate", workspace_b.path().to_path_buf());
    runner
        .run(
            SubagentRequest::new("helper", "other workspace task", task_b),
            ctx.clone(),
        )
        .await
        .expect("fresh child turn in another cwd");

    // Первый ребёнок не тронут — resume по его task_id продолжает работать.
    let resumed = runner
        .run(
            SubagentRequest::new("helper", "continue first", task_a)
                .with_metadata(json!({ "task_id": first_task_id.to_string() })),
            ctx,
        )
        .await
        .expect("resume of first child must survive foreign-cwd fresh task");
    assert_eq!(resumed.child_thread_id, Some(first_task_id));
}
