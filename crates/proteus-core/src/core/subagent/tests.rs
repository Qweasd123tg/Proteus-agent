//! Runner-level тесты `SequentialSubagentRunner`: конфиг/роли, изоляция
//! контекста, дочерний цикл, resumable snapshots и cancel-safety.
//! Unit-тесты helpers живут рядом с кодом в `child_loop` и `resumable`.

use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};

use super::*;
use crate::{
    contracts::{
        CancellationToken, EventEmitter, ModelClient, PolicyContext, PolicyVisibilityContext,
        SubagentIsolation, SubagentLimits, ToolRegistry,
    },
    core::{HeadlessApprovalTransport, HeadlessUserInputTransport, InMemoryEventStore},
    domain::{
        AgentTask, CacheHints, ModelRef, PolicyDecision, ReasoningConfig, ToolCall, new_session_id,
        new_thread_id, new_turn_id,
    },
    model_standard::{CanonicalModelRequest, CanonicalModelResponse},
    stubs::{
        AllVisibleToolExposure, EmptyContextBuilder, FakeModelClient, NoCompactor, NoMemory,
        NoSubagent, NullPatchApplier, NullSearch,
    },
    tools::RememberFactTool,
};

struct AllowAllPolicy;

impl crate::contracts::ApprovalPolicy for AllowAllPolicy {
    fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        PolicyDecision::Allow
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

struct FailingModelClient;

#[async_trait]
impl ModelClient for FailingModelClient {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("failing")
    }

    fn capabilities(&self, model: &ModelRef) -> crate::model_standard::ModelCapabilities {
        FakeModelClient::default().capabilities(model)
    }

    async fn complete(&self, _request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        Err(anyhow!("model boom"))
    }

    async fn stream(
        &self,
        _request: CanonicalModelRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<crate::model_standard::ModelStreamEvent>>
                    + Send,
            >,
        >,
    > {
        Err(anyhow!("model stream boom"))
    }
}

/// Отменяет переданный токен при первом `complete` и возвращает ошибку —
/// эмулирует cancel родительского turn-а во время model call ребёнка.
struct CancellingModelClient {
    token: CancellationToken,
}

#[async_trait]
impl ModelClient for CancellingModelClient {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("cancelling")
    }

    fn capabilities(&self, model: &ModelRef) -> crate::model_standard::ModelCapabilities {
        FakeModelClient::default().capabilities(model)
    }

    async fn complete(&self, _request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        self.token.cancel();
        Err(anyhow!("turn canceled by client"))
    }

    async fn stream(
        &self,
        _request: CanonicalModelRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<crate::model_standard::ModelStreamEvent>>
                    + Send,
            >,
        >,
    > {
        Err(anyhow!("model stream boom"))
    }
}

#[derive(Default)]
struct RecordingFakeModelClient {
    inner: FakeModelClient,
    histories: StdMutex<Vec<Vec<CanonicalMessage>>>,
    metadatas: StdMutex<Vec<Value>>,
    caches: StdMutex<Vec<CacheHints>>,
}

impl RecordingFakeModelClient {
    fn histories(&self) -> Vec<Vec<CanonicalMessage>> {
        self.histories.lock().expect("histories lock").clone()
    }

    fn metadatas(&self) -> Vec<Value> {
        self.metadatas.lock().expect("metadatas lock").clone()
    }

    fn caches(&self) -> Vec<CacheHints> {
        self.caches.lock().expect("caches lock").clone()
    }
}

#[async_trait]
impl ModelClient for RecordingFakeModelClient {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        self.inner.id()
    }

    fn capabilities(&self, model: &ModelRef) -> crate::model_standard::ModelCapabilities {
        self.inner.capabilities(model)
    }

    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        self.histories
            .lock()
            .expect("histories lock")
            .push(request.messages.clone());
        self.metadatas
            .lock()
            .expect("metadatas lock")
            .push(request.metadata.clone());
        self.caches
            .lock()
            .expect("caches lock")
            .push(request.cache.clone());
        self.inner.complete(request).await
    }

    async fn stream(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<crate::model_standard::ModelStreamEvent>>
                    + Send,
            >,
        >,
    > {
        self.inner.stream(request).await
    }
}

fn test_runtime_context(events: Arc<InMemoryEventStore>) -> RuntimeContext {
    test_runtime_context_with_model(events, Arc::new(FakeModelClient::default()))
}

fn test_runtime_context_with_model<M>(
    events: Arc<InMemoryEventStore>,
    model: Arc<M>,
) -> RuntimeContext
where
    M: ModelClient + 'static,
{
    let mut tools = ToolRegistry::new();
    tools
        .register(RememberFactTool::new(Arc::new(NoMemory)))
        .expect("register remember_fact");
    RuntimeContext::new(
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        ModelRef::new("fake", "fake-tool-model"),
        ReasoningConfig::default(),
        120_000,
        30_000,
        Arc::new(EventEmitter::new(events)),
        model,
        Arc::new(NullSearch),
        Arc::new(NoMemory),
        Arc::new(EmptyContextBuilder),
        tools,
        Arc::new(AllowAllPolicy),
        Arc::new(HeadlessApprovalTransport),
        Arc::new(HeadlessUserInputTransport),
        Arc::new(NullPatchApplier),
        Arc::new(NoCompactor),
        Arc::new(AllVisibleToolExposure),
        Arc::new(NoSubagent),
    )
}

fn explorer_config() -> Value {
    json!({
        "roles": [
            {
                "name": "explore",
                "description": "Read-only exploration",
                "prompt": "You are a read-only explorer.",
                "max_iterations": 15,
                "timeout_ms": 60000,
                "max_summary_bytes": 2048,
                "exposure_phase": "explore_phase",
                "tools": ["remember_fact"]
            },
            {
                "name": "reviewer",
                "description": "Review changes",
                "prompt": "You review diffs."
            }
        ],
        "max_depth": 2
    })
}

#[test]
fn parses_roles_and_limits_from_config() {
    let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
    let roles = runner.roles();

    assert_eq!(roles.len(), 2);
    let explore = &roles[0];
    assert_eq!(explore.name, "explore");
    assert_eq!(explore.description, "Read-only exploration");
    assert_eq!(explore.prompt, "You are a read-only explorer.");
    assert_eq!(explore.limits.max_iterations, 15);
    assert_eq!(explore.limits.timeout_ms, Some(60000));
    assert_eq!(explore.limits.max_summary_bytes, Some(2048));
    assert_eq!(explore.effective_exposure_phase(), "explore_phase");
    assert_eq!(
        explore
            .config
            .get("tools")
            .and_then(Value::as_array)
            .unwrap(),
        &vec![json!("remember_fact")]
    );

    let reviewer = &roles[1];
    assert_eq!(
        reviewer.limits.max_iterations,
        SubagentLimits::default().max_iterations
    );
    assert_eq!(reviewer.limits.timeout_ms, None);
    assert_eq!(reviewer.effective_exposure_phase(), "subagent:reviewer");
    assert_eq!(runner.inner.max_depth, 2);
}

#[test]
fn missing_config_means_no_roles() {
    let runner = SequentialSubagentRunner::from_config(Value::Null).unwrap();
    assert!(runner.roles().is_empty());
    assert_eq!(runner.inner.max_depth, 1);
}

#[test]
fn duplicate_role_names_are_rejected() {
    let error = SequentialSubagentRunner::from_config(json!({
        "roles": [
            { "name": "explore", "description": "a", "prompt": "p" },
            { "name": "explore", "description": "b", "prompt": "p" }
        ]
    }))
    .unwrap_err();
    assert!(error.to_string().contains("duplicate subagent role"));
}

#[test]
fn parses_worktree_isolation_from_config() {
    let runner = SequentialSubagentRunner::from_config(json!({
        "roles": [
            { "name": "coder", "description": "d", "prompt": "p", "isolation": "worktree" },
            { "name": "explore", "description": "d", "prompt": "p" }
        ]
    }))
    .unwrap();
    let roles = runner.roles();
    assert_eq!(roles[0].isolation, SubagentIsolation::Worktree);
    assert_eq!(roles[1].isolation, SubagentIsolation::None);
}

#[test]
fn unknown_isolation_value_is_rejected() {
    let error = SequentialSubagentRunner::from_config(json!({
        "roles": [
            { "name": "coder", "description": "d", "prompt": "p", "isolation": "container" }
        ]
    }))
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("unknown isolation value"), "{message}");
    assert!(message.contains("coder"), "{message}");
}

#[test]
fn markdown_role_is_loaded_from_roles_dir() {
    let workspace = tempfile::tempdir().expect("workspace");
    let roles_dir = workspace.path().join("roles");
    std::fs::create_dir(&roles_dir).expect("roles dir");
    std::fs::write(
        roles_dir.join("markdown.md"),
        "---\n\
description: Markdown role\n\
exposure_phase: md_phase\n\
tools:\n\
  - remember_fact\n\
isolation: worktree\n\
max_iterations: 3\n\
timeout_ms: 42\n\
max_summary_bytes: 7\n\
---\n\
\n\
Markdown prompt.\n",
    )
    .expect("role file");

    let runner = SequentialSubagentRunner::from_config_with_cwd(
        json!({ "roles_dir": "roles" }),
        workspace.path(),
    )
    .unwrap();
    let roles = runner.roles();

    assert_eq!(roles.len(), 1);
    let role = &roles[0];
    assert_eq!(role.name, "markdown");
    assert_eq!(role.description, "Markdown role");
    assert_eq!(role.prompt, "Markdown prompt.");
    assert_eq!(role.limits.max_iterations, 3);
    assert_eq!(role.limits.timeout_ms, Some(42));
    assert_eq!(role.limits.max_summary_bytes, Some(7));
    assert_eq!(role.effective_exposure_phase(), "md_phase");
    assert_eq!(role.isolation, SubagentIsolation::Worktree);
    assert_eq!(
        role.config.get("tools").and_then(Value::as_array).unwrap(),
        &vec![json!("remember_fact")]
    );
}

#[test]
fn invalid_markdown_frontmatter_is_rejected_with_file_name() {
    let workspace = tempfile::tempdir().expect("workspace");
    let roles_dir = workspace.path().join("roles");
    std::fs::create_dir(&roles_dir).expect("roles dir");
    std::fs::write(
        roles_dir.join("bad.md"),
        "---\ndescription: [\n---\nPrompt\n",
    )
    .expect("role file");

    let error = SequentialSubagentRunner::from_config_with_cwd(
        json!({ "roles_dir": "roles" }),
        workspace.path(),
    )
    .unwrap_err();

    assert!(error.to_string().contains("bad.md"), "{error:#}");
}

#[test]
fn markdown_role_duplicate_with_inline_role_is_rejected() {
    let workspace = tempfile::tempdir().expect("workspace");
    let roles_dir = workspace.path().join("roles");
    std::fs::create_dir(&roles_dir).expect("roles dir");
    std::fs::write(
        roles_dir.join("explore.md"),
        "---\ndescription: Markdown role\n---\nMarkdown prompt\n",
    )
    .expect("role file");

    let error = SequentialSubagentRunner::from_config_with_cwd(
        json!({
            "roles": [
                { "name": "explore", "description": "Inline", "prompt": "prompt" }
            ],
            "roles_dir": "roles"
        }),
        workspace.path(),
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("duplicate subagent role: explore"),
        "{error:#}"
    );
}

/// Изоляция turn-scoped grants структурная: ребёнок стартует с пустыми
/// grants (escalated_exec родителя не протекает) и его собственные
/// grants не видны родительскому ходу.
#[test]
fn child_context_isolates_turn_grants_and_labels_thread() {
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events);
    ctx.turn_grants.grant(["escalated_exec".to_owned()]);

    let child_thread_id = new_thread_id();
    let child_ctx = child_context(&ctx, child_thread_id, "explore");

    assert_eq!(child_ctx.thread_id, child_thread_id);
    assert_eq!(child_ctx.thread_label.as_deref(), Some("explore"));
    assert!(
        child_ctx.turn_grants.snapshot().is_empty(),
        "parent grants must not leak into the child"
    );

    child_ctx.turn_grants.grant(["child_grant".to_owned()]);
    assert_eq!(
        ctx.turn_grants.snapshot(),
        vec!["escalated_exec"],
        "child grants must not leak back into the parent"
    );
}

/// Ребёнок живёт на child-токене: cancel родителя каскадится ребёнку,
/// а cancel ребёнка не отменяет родительский turn (groundwork parallel
/// subagents: одного ребёнка можно снять, не трогая остальных).
#[test]
fn child_context_uses_child_cancellation_token() {
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events);

    let child_ctx = child_context(&ctx, new_thread_id(), "explore");
    child_ctx.cancellation.cancel();
    assert!(child_ctx.is_cancelled());
    assert!(
        !ctx.is_cancelled(),
        "child cancel must not cancel the parent turn"
    );

    let second_child = child_context(&ctx, new_thread_id(), "explore");
    ctx.cancellation.cancel();
    assert!(
        second_child.is_cancelled(),
        "parent cancel must cascade to the child"
    );
}

#[tokio::test]
async fn depth_limit_is_enforced() {
    let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events);
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let request = SubagentRequest::new("explore", "look around", task)
        .with_metadata(json!({ "subagent_depth": 2 }));
    let error = runner.run(request, ctx).await.unwrap_err();

    assert!(
        error.to_string().contains("subagent depth limit reached"),
        "{error:#}"
    );
}

#[tokio::test]
async fn unknown_role_is_rejected() {
    let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events);
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let error = runner
        .run(SubagentRequest::new("mystery", "look", task), ctx)
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("unknown subagent role: mystery"),
        "{error:#}"
    );
}

#[tokio::test]
async fn run_emits_errored_finished_when_model_errors() {
    let runner = SequentialSubagentRunner::from_config(json!({
        "roles": [
            {
                "name": "explore",
                "description": "Explore",
                "prompt": "prompt",
                "max_iterations": 3
            }
        ]
    }))
    .unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context_with_model(events.clone(), Arc::new(FailingModelClient));
    let parent_thread_id = ctx.thread_id;
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let error = runner
        .run(SubagentRequest::new("explore", "look", task), ctx)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("model boom"), "{error:#}");

    let envelopes = events.envelopes().await;
    let started_child_thread_id = envelopes
        .iter()
        .find_map(|envelope| match &envelope.event {
            Event::SubagentStarted {
                child_thread_id, ..
            } => {
                assert_eq!(envelope.thread_id, parent_thread_id);
                Some(*child_thread_id)
            }
            _ => None,
        })
        .expect("SubagentStarted");

    let finished = envelopes
        .iter()
        .find(|envelope| matches!(envelope.event, Event::SubagentFinished { .. }))
        .expect("SubagentFinished");
    assert_eq!(finished.thread_id, parent_thread_id);
    match &finished.event {
        Event::SubagentFinished {
            role,
            status,
            iterations,
            child_thread_id,
        } => {
            assert_eq!(role, "explore");
            assert_eq!(status, "errored");
            assert_eq!(*iterations, 0);
            assert_eq!(*child_thread_id, started_child_thread_id);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn child_loop_runs_tool_call_round_trip_with_fake_model() {
    let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events.clone());
    let parent_thread_id = ctx.thread_id;
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("remember", cwd.path().to_path_buf());

    // FakeModelClient: "remember_fact <text>" → tool call remember_fact,
    // после tool result — финальный текст без tool calls.
    let result = runner
        .run(
            SubagentRequest::new("explore", "remember_fact user prefers tabs", task)
                .with_description("remember tabs"),
            ctx,
        )
        .await
        .unwrap();

    assert_eq!(result.status, SubagentStatus::Completed);
    assert_eq!(result.iterations, 2);
    assert!(
        result.summary.contains("Fake final answer"),
        "summary: {}",
        result.summary
    );
    let child_thread_id = result.child_thread_id.expect("child thread id");
    assert_ne!(child_thread_id, parent_thread_id);

    let envelopes = events.envelopes().await;
    let started = envelopes
        .iter()
        .find(|envelope| matches!(envelope.event, Event::SubagentStarted { .. }))
        .expect("SubagentStarted");
    assert_eq!(started.thread_id, parent_thread_id);
    match &started.event {
        Event::SubagentStarted {
            role,
            description,
            child_thread_id: event_child,
        } => {
            assert_eq!(role, "explore");
            assert_eq!(description.as_deref(), Some("remember tabs"));
            assert_eq!(*event_child, child_thread_id);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    // Tool события ребёнка идут под child_thread_id.
    let tool_started = envelopes
        .iter()
        .find(|envelope| matches!(envelope.event, Event::ToolCallRequested { .. }))
        .expect("ToolCallRequested");
    assert_eq!(tool_started.thread_id, child_thread_id);

    let finished = envelopes
        .iter()
        .find(|envelope| matches!(envelope.event, Event::SubagentFinished { .. }))
        .expect("SubagentFinished");
    assert_eq!(finished.thread_id, parent_thread_id);
    match &finished.event {
        Event::SubagentFinished {
            role,
            status,
            iterations,
            child_thread_id: event_child,
        } => {
            assert_eq!(role, "explore");
            assert_eq!(status, "completed");
            assert_eq!(*iterations, 2);
            assert_eq!(*event_child, child_thread_id);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn child_loop_model_requests_suppress_stream_deltas() {
    let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let model = Arc::new(RecordingFakeModelClient::default());
    let ctx = test_runtime_context_with_model(events, model.clone());
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    runner
        .run(SubagentRequest::new("explore", "look around", task), ctx)
        .await
        .unwrap();

    // Delta-контекст ModelService указывает на родительский ход: без
    // подавления стрим ребёнка утёк бы в родительский транскрипт.
    let metadatas = model.metadatas();
    assert!(!metadatas.is_empty());
    assert!(
        metadatas
            .iter()
            .all(|metadata| metadata["suppress_stream_deltas"] == json!(true))
    );
}

/// Дочерние запросы должны включать prompt cache: append-only история
/// ребёнка без cache hints и стабильного ключа перепрефилливалась почти
/// по полной цене на каждой итерации (dogfood-находка 2026-07-06).
#[tokio::test]
async fn child_loop_model_requests_enable_prompt_cache() {
    let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let model = Arc::new(RecordingFakeModelClient::default());
    let ctx = test_runtime_context_with_model(events, model.clone());
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    runner
        .run(SubagentRequest::new("explore", "look around", task), ctx)
        .await
        .unwrap();

    let metadatas = model.metadatas();
    assert!(!metadatas.is_empty());
    let first_key = metadatas[0]["prompt_cache_key"]
        .as_str()
        .expect("prompt_cache_key present");
    assert!(first_key.starts_with("proteus:subagent:"));
    assert!(
        metadatas
            .iter()
            .all(|metadata| metadata["prompt_cache_key"].as_str() == Some(first_key)),
        "cache key must be stable across child iterations"
    );

    let caches = model.caches();
    assert!(!caches.is_empty());
    assert!(
        caches
            .iter()
            .all(|cache| cache.cache_instructions && cache.cache_context)
    );
}

#[tokio::test]
async fn resumable_task_id_round_trips_history_and_thread_id() {
    let runner = SequentialSubagentRunner::from_config(json!({
        "roles": [
            {
                "name": "explore",
                "description": "Explore",
                "prompt": "prompt",
                "max_iterations": 5,
                "tools": ["remember_fact"]
            }
        ]
    }))
    .unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let model = Arc::new(RecordingFakeModelClient::default());
    let ctx = test_runtime_context_with_model(events.clone(), model.clone());
    let parent_thread_id = ctx.thread_id;
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let first = runner
        .run(
            SubagentRequest::new("explore", "first prompt", task.clone()),
            ctx.clone(),
        )
        .await
        .unwrap();
    assert_eq!(first.status, SubagentStatus::Completed);
    assert_eq!(first.iterations, 1);
    assert_eq!(first.metadata["resumable"], json!(true));
    let child_thread_id = first.child_thread_id.expect("child thread id");

    let second = runner
        .run(
            SubagentRequest::new("explore", "remember_fact resumed fact", task)
                .with_metadata(json!({ "task_id": child_thread_id.to_string() })),
            ctx,
        )
        .await
        .unwrap();

    assert_eq!(second.child_thread_id, Some(child_thread_id));
    assert_eq!(second.status, SubagentStatus::Completed);
    assert_eq!(second.iterations, 2);
    assert!(
        second
            .summary
            .contains("Fake final answer after tool result")
    );
    assert_eq!(second.metadata["resumable"], json!(true));

    let histories = model.histories();
    assert_eq!(histories.len(), 3);
    assert_eq!(histories[0].len(), 2);
    assert_eq!(histories[1].len(), 4);
    assert_eq!(histories[1][0].role, MessageRole::System);
    assert_eq!(histories[1][1].role, MessageRole::User);
    assert_eq!(histories[1][2].role, MessageRole::Assistant);
    assert_eq!(histories[1][3].role, MessageRole::User);

    let envelopes = events.envelopes().await;
    let started_with_child = envelopes
        .iter()
        .filter(|envelope| match &envelope.event {
            Event::SubagentStarted {
                child_thread_id: event_child,
                ..
            } => {
                assert_eq!(envelope.thread_id, parent_thread_id);
                *event_child == child_thread_id
            }
            _ => false,
        })
        .count();
    assert_eq!(started_with_child, 2);
    assert!(
        envelopes.iter().any(|envelope| {
            envelope.thread_id == child_thread_id
                && matches!(envelope.event, Event::ToolCallRequested { .. })
        }),
        "expected resumed tool event under original child thread"
    );
}

/// Cancel не должен терять частичную работу ребёнка: snapshot сохраняется
/// и при `Cancelled`, работу можно продолжить по тому же `task_id`
/// (кластер 2 аудита, dogfood-находка (d) 2026-07-06).
#[tokio::test]
async fn cancelled_child_saves_resumable_snapshot_and_resumes() {
    let runner = SequentialSubagentRunner::from_config(json!({
        "roles": [
            { "name": "explore", "description": "Explore", "prompt": "prompt" }
        ]
    }))
    .unwrap();
    let events = Arc::new(InMemoryEventStore::new());

    // Первый прогон: модель отменяет turn во время complete → Cancelled.
    let mut cancelling_ctx = test_runtime_context(events.clone());
    let session_id = cancelling_ctx.session_id;
    cancelling_ctx.model = Arc::new(CancellingModelClient {
        token: cancelling_ctx.cancellation.clone(),
    });
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let cancelled = runner
        .run(
            SubagentRequest::new("explore", "long exploration", task.clone()),
            cancelling_ctx,
        )
        .await
        .unwrap();

    assert_eq!(cancelled.status, SubagentStatus::Cancelled);
    assert_eq!(
        cancelled.metadata["resumable"],
        json!(true),
        "cancelled child must keep a resumable snapshot"
    );
    let task_id = cancelled.child_thread_id.expect("child thread id");

    // Второй прогон: тот же session_id, свежий токен — resume по task_id.
    let mut resume_ctx = test_runtime_context(events);
    resume_ctx.session_id = session_id;
    let resumed = runner
        .run(
            SubagentRequest::new("explore", "continue exploring", task)
                .with_metadata(json!({ "task_id": task_id.to_string() })),
            resume_ctx,
        )
        .await
        .unwrap();

    assert_eq!(resumed.status, SubagentStatus::Completed);
    assert_eq!(resumed.child_thread_id, Some(task_id));
}

#[tokio::test]
async fn unknown_task_id_is_rejected() {
    let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events);
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let error = runner
        .run(
            SubagentRequest::new("explore", "look", task)
                .with_metadata(json!({ "task_id": new_thread_id().to_string() })),
            ctx,
        )
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unknown task_id (expired or from another session)"),
        "{error:#}"
    );
}

#[tokio::test]
async fn task_id_from_another_session_is_rejected() {
    let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events.clone());
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let first = runner
        .run(SubagentRequest::new("explore", "first", task.clone()), ctx)
        .await
        .unwrap();
    let task_id = first.child_thread_id.expect("child thread id").to_string();

    let other_ctx = test_runtime_context(events);
    let error = runner
        .run(
            SubagentRequest::new("explore", "second", task)
                .with_metadata(json!({ "task_id": task_id })),
            other_ctx,
        )
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unknown task_id (expired or from another session)"),
        "{error:#}"
    );
}

#[tokio::test]
async fn resumable_store_evicts_least_recently_used_snapshot() {
    let runner = SequentialSubagentRunner::from_config(json!({
        "roles": [
            { "name": "explore", "description": "Explore", "prompt": "prompt" }
        ],
        "max_resumable": 1
    }))
    .unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events.clone());
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let first = runner
        .run(SubagentRequest::new("explore", "first", task.clone()), ctx)
        .await
        .unwrap();
    let first_task_id = first.child_thread_id.expect("child thread id").to_string();

    let ctx = test_runtime_context(events.clone());
    runner
        .run(SubagentRequest::new("explore", "second", task.clone()), ctx)
        .await
        .unwrap();

    let ctx = test_runtime_context(events);
    let error = runner
        .run(
            SubagentRequest::new("explore", "resume first", task)
                .with_metadata(json!({ "task_id": first_task_id })),
            ctx,
        )
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unknown task_id (expired or from another session)"),
        "{error:#}"
    );
}

/// Разблокирует `complete` только когда `expected` запросов пришли
/// одновременно: sequential-исполнение батча повисло бы на первом.
struct BarrierModelClient {
    barrier: tokio::sync::Barrier,
}

#[async_trait]
impl ModelClient for BarrierModelClient {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("barrier")
    }

    fn capabilities(&self, model: &ModelRef) -> crate::model_standard::ModelCapabilities {
        FakeModelClient::default().capabilities(model)
    }

    async fn complete(&self, _request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        self.barrier.wait().await;
        Ok(CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "concurrent done"),
            Vec::new(),
            crate::model_standard::FinishReason::Stop,
        ))
    }

    async fn stream(
        &self,
        _request: CanonicalModelRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<crate::model_standard::ModelStreamEvent>>
                    + Send,
            >,
        >,
    > {
        Err(anyhow!("not used"))
    }
}

/// Виснет навсегда, если prompt содержит "block me" (снимается только
/// отменой через select в child_loop); иначе отвечает сразу.
struct SelectiveBlockingModelClient;

#[async_trait]
impl ModelClient for SelectiveBlockingModelClient {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("selective")
    }

    fn capabilities(&self, model: &ModelRef) -> crate::model_standard::ModelCapabilities {
        FakeModelClient::default().capabilities(model)
    }

    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        let should_block = request.messages.iter().any(|message| {
            message.parts.iter().any(|part| {
                matches!(
                    part,
                    crate::model_standard::ContentPart::Text { text } if text.contains("block me")
                )
            })
        });
        if should_block {
            futures_util::future::pending::<()>().await;
        }
        Ok(CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "quick done"),
            Vec::new(),
            crate::model_standard::FinishReason::Stop,
        ))
    }

    async fn stream(
        &self,
        _request: CanonicalModelRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<crate::model_standard::ModelStreamEvent>>
                    + Send,
            >,
        >,
    > {
        Err(anyhow!("not used"))
    }
}

fn parallel_roles_config() -> Value {
    json!({
        "roles": [
            {
                "name": "explore",
                "description": "Read-only exploration",
                "prompt": "prompt",
                "parallel_safe": true
            }
        ]
    })
}

#[test]
fn parallel_safe_flag_round_trips_from_config() {
    let runner = SequentialSubagentRunner::from_config(json!({
        "roles": [
            { "name": "explore", "description": "a", "prompt": "p", "parallel_safe": true },
            { "name": "writer", "description": "b", "prompt": "p" }
        ]
    }))
    .unwrap();
    let roles = runner.roles();
    assert!(roles[0].parallel_safe);
    assert!(!roles[1].parallel_safe);
}

/// Два ребёнка действительно работают конкурентно: каждый блокируется на
/// барьере в model call и проходит его только вместе с соседом.
#[tokio::test]
async fn spawned_children_run_concurrently() {
    let runner = SequentialSubagentRunner::from_config(parallel_roles_config()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let model = Arc::new(BarrierModelClient {
        barrier: tokio::sync::Barrier::new(2),
    });
    let ctx = test_runtime_context_with_model(events.clone(), model);
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let first = runner
        .spawn(
            SubagentRequest::new("explore", "first branch", task.clone()),
            ctx.clone(),
        )
        .await
        .unwrap();
    let second = runner
        .spawn(SubagentRequest::new("explore", "second branch", task), ctx)
        .await
        .unwrap();
    assert_ne!(first.spawn_id, second.spawn_id);
    assert_ne!(first.child_thread_id, second.child_thread_id);

    // Started обоих детей эмитятся до wait: модель видит оба запуска сразу.
    let started = events
        .envelopes()
        .await
        .iter()
        .filter(|envelope| matches!(envelope.event, Event::SubagentStarted { .. }))
        .count();
    assert_eq!(started, 2);

    let first_result = tokio::time::timeout(Duration::from_secs(5), runner.wait(&first))
        .await
        .expect("first wait must not hang")
        .unwrap();
    let second_result = tokio::time::timeout(Duration::from_secs(5), runner.wait(&second))
        .await
        .expect("second wait must not hang")
        .unwrap();

    assert_eq!(first_result.status, SubagentStatus::Completed);
    assert_eq!(second_result.status, SubagentStatus::Completed);
    assert!(first_result.summary.contains("concurrent done"));
    assert!(second_result.summary.contains("concurrent done"));
}

/// Cancel по handle снимает только адресованного ребёнка: сосед завершается
/// штатно, родительский turn не отменяется, у отменённого сохраняется
/// resumable snapshot.
#[tokio::test]
async fn cancel_by_handle_cancels_only_target_child() {
    let runner = SequentialSubagentRunner::from_config(parallel_roles_config()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context_with_model(events, Arc::new(SelectiveBlockingModelClient));
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let blocked = runner
        .spawn(
            SubagentRequest::new("explore", "please block me", task.clone()),
            ctx.clone(),
        )
        .await
        .unwrap();
    let quick = runner
        .spawn(
            SubagentRequest::new("explore", "answer fast", task),
            ctx.clone(),
        )
        .await
        .unwrap();

    let quick_result = tokio::time::timeout(Duration::from_secs(5), runner.wait(&quick))
        .await
        .expect("quick wait must not hang")
        .unwrap();
    assert_eq!(quick_result.status, SubagentStatus::Completed);

    runner.cancel(&blocked).await.unwrap();
    let blocked_result = tokio::time::timeout(Duration::from_secs(5), runner.wait(&blocked))
        .await
        .expect("cancelled wait must not hang")
        .unwrap();
    assert_eq!(blocked_result.status, SubagentStatus::Cancelled);
    assert_eq!(
        blocked_result.metadata["resumable"],
        json!(true),
        "cancelled child keeps resumable snapshot"
    );
    assert!(
        !ctx.is_cancelled(),
        "cancelling a child must not cancel the parent turn"
    );
}

#[tokio::test]
async fn wait_consumes_handle_once() {
    let runner = SequentialSubagentRunner::from_config(parallel_roles_config()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events);
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let handle = runner
        .spawn(SubagentRequest::new("explore", "look", task), ctx)
        .await
        .unwrap();
    runner.wait(&handle).await.unwrap();

    let error = runner.wait(&handle).await.unwrap_err();
    assert!(
        error.to_string().contains("unknown subagent spawn_id"),
        "{error:#}"
    );
    let error = runner.cancel(&handle).await.unwrap_err();
    assert!(
        error.to_string().contains("unknown subagent spawn_id"),
        "{error:#}"
    );
}

/// Ошибки подготовки возвращаются из spawn до SubagentStarted.
#[tokio::test]
async fn spawn_rejects_unknown_role_before_started_event() {
    let runner = SequentialSubagentRunner::from_config(parallel_roles_config()).unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context(events.clone());
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let error = runner
        .spawn(SubagentRequest::new("mystery", "look", task), ctx)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unknown subagent role"));
    assert!(
        events
            .envelopes()
            .await
            .iter()
            .all(|envelope| !matches!(envelope.event, Event::SubagentStarted { .. })),
        "no Started event for failed spawn"
    );
}

#[tokio::test]
async fn spawn_respects_max_parallel_cap() {
    let runner = SequentialSubagentRunner::from_config(json!({
        "roles": [
            {
                "name": "explore",
                "description": "Read-only exploration",
                "prompt": "prompt",
                "parallel_safe": true
            }
        ],
        "max_parallel": 1
    }))
    .unwrap();
    let events = Arc::new(InMemoryEventStore::new());
    let ctx = test_runtime_context_with_model(events, Arc::new(SelectiveBlockingModelClient));
    let cwd = tempfile::tempdir().expect("workspace");
    let task = AgentTask::new("explore", cwd.path().to_path_buf());

    let blocked = runner
        .spawn(
            SubagentRequest::new("explore", "please block me", task.clone()),
            ctx.clone(),
        )
        .await
        .unwrap();

    let error = runner
        .spawn(
            SubagentRequest::new("explore", "one more", task),
            ctx.clone(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("max_parallel"), "{error:#}");

    runner.cancel(&blocked).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), runner.wait(&blocked))
        .await
        .expect("wait must not hang")
        .unwrap();
    assert_eq!(result.status, SubagentStatus::Cancelled);
}
