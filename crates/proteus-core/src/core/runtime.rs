use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use tokio::time::{Duration, timeout};

use crate::{
    contracts::{
        ApprovalTransport, CancellationToken, EventEmitter, EventSink, MemoryPolicyInput,
        ToolSource, UserInputTransport,
    },
    core::{
        AppConfig, BuiltinRegistry, CachedApprovalTransport, HeadlessApprovalTransport,
        HeadlessUserInputTransport, JsonlEventStore, SessionStore,
    },
    domain::{
        AgentOutput, AgentTask, Event, EventContext, ModelRef, PermissionMode, ReasoningConfig,
        SessionId, ThreadId, ToolSpec, new_session_id, new_thread_id, new_turn_id,
    },
    model_standard::CanonicalMessage,
};

pub struct AgentRuntime {
    services: RuntimeServices,
    session: SessionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModuleEpoch(u64);

impl ModuleEpoch {
    pub fn initial() -> Self {
        Self(0)
    }

    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone)]
pub struct RuntimeSnapshot {
    pub epoch: ModuleEpoch,
    pub registry: BuiltinRegistry,
}

impl RuntimeSnapshot {
    pub fn new(epoch: ModuleEpoch, registry: BuiltinRegistry) -> Self {
        Self { epoch, registry }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeReloadReport {
    pub old_epoch: u64,
    pub new_epoch: u64,
    pub tool_names: Vec<String>,
}

struct RuntimeServices {
    cwd: PathBuf,
    snapshot: RwLock<RuntimeSnapshot>,
    reload_lock: Mutex<()>,
    events: Arc<EventEmitter>,
    approval: Arc<dyn ApprovalTransport>,
    user_input: Arc<dyn UserInputTransport>,
    permission_mode: RwLock<PermissionMode>,
    model_ref: RwLock<ModelRef>,
    reasoning: RwLock<ReasoningConfig>,
    default_reasoning: ReasoningConfig,
}

struct SessionState {
    session_id: SessionId,
    thread_id: ThreadId,
    run_lock: Mutex<()>,
    session_started: Mutex<bool>,
    history: Mutex<Vec<CanonicalMessage>>,
    session_store: Option<SessionStore>,
}

impl SessionState {
    fn new(
        session_id: SessionId,
        thread_id: ThreadId,
        session_store: Option<SessionStore>,
        history: Vec<CanonicalMessage>,
        session_started: bool,
    ) -> Self {
        Self {
            session_id,
            thread_id,
            run_lock: Mutex::new(()),
            session_started: Mutex::new(session_started),
            history: Mutex::new(history),
            session_store,
        }
    }
}

impl AgentRuntime {
    /// Entry-point for composing a runtime from replaceable parts without
    /// accumulating constructor overloads. Start with
    /// `AgentRuntime::builder(config, cwd)` and chain `.with_*` methods.
    pub fn builder(config: AppConfig, cwd: PathBuf) -> AgentRuntimeBuilder {
        AgentRuntimeBuilder::new(config, cwd)
    }

    pub fn new(config: AppConfig, cwd: PathBuf) -> Result<Self> {
        let config_path = AppConfig::default_user_config_path();
        Self::builder(config, cwd)
            .with_config_path(config_path.as_deref())
            .build()
    }

    pub fn new_with_config_path(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&std::path::Path>,
    ) -> Result<Self> {
        Self::builder(config, cwd)
            .with_config_path(config_path)
            .build()
    }

    pub fn new_with_config_path_and_approval_transport(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&std::path::Path>,
        approval: Arc<dyn ApprovalTransport>,
    ) -> Result<Self> {
        Self::builder(config, cwd)
            .with_config_path(config_path)
            .with_approval(approval)
            .build()
    }

    pub fn with_event_sink(
        config: AppConfig,
        cwd: PathBuf,
        event_sink: Arc<dyn EventSink>,
    ) -> Result<Self> {
        Self::builder(config, cwd)
            .with_event_sink(event_sink)
            .build()
    }

    pub fn with_event_sink_and_approval_transport(
        config: AppConfig,
        cwd: PathBuf,
        event_sink: Arc<dyn EventSink>,
        approval: Arc<dyn ApprovalTransport>,
    ) -> Result<Self> {
        Self::builder(config, cwd)
            .with_event_sink(event_sink)
            .with_approval(approval)
            .build()
    }

    pub async fn run(&self, text: String) -> Result<AgentOutput> {
        self.run_with_cancellation(text, CancellationToken::new())
            .await
    }

    pub async fn run_with_cancellation(
        &self,
        text: String,
        cancellation: CancellationToken,
    ) -> Result<AgentOutput> {
        let _run_guard = self.session.run_lock.lock().await;
        let snapshot = self.snapshot().await;
        self.ensure_session_started_with_snapshot(&snapshot).await?;
        let turn_id = new_turn_id();
        if cancellation.is_cancelled() {
            anyhow::bail!("turn canceled by client");
        }
        let event_context = EventContext::new(
            self.session.session_id,
            self.session.thread_id,
            Some(turn_id),
        );
        self.services
            .events
            .emit(
                event_context,
                Event::TurnStarted {
                    session_id: self.session.session_id,
                    thread_id: self.session.thread_id,
                    turn_id,
                },
            )
            .await?;
        let task = AgentTask::new(text, self.services.cwd.clone());
        // Выставляем delta event context для ModelService, чтобы
        // streaming TextDelta/ToolArgsDelta/ReasoningDelta эмитились с
        // правильным envelope (session/thread/turn). Без этого дельты
        // тихо дропаются (штатное поведение без runtime).
        if let Some(service) = &snapshot.registry.model_service {
            service.set_event_context(crate::core::DeltaEventContext {
                emitter: Some(self.services.events.clone()),
                session_id: Some(self.session.session_id),
                thread_id: Some(self.session.thread_id),
                turn_id: Some(turn_id),
            });
        }
        let permission_mode = *self.services.permission_mode.read().await;
        let model_ref = self.services.model_ref.read().await.clone();
        let reasoning = self.services.reasoning.read().await.clone();
        let mut runtime_context = snapshot.registry.runtime_context_with_user_input(
            self.session.session_id,
            self.session.thread_id,
            turn_id,
            self.services.events.clone(),
            self.services.approval.clone(),
            self.services.user_input.clone(),
            permission_mode,
        );
        runtime_context.model_ref = model_ref;
        runtime_context.reasoning = reasoning;
        let runtime_context = runtime_context.with_cancellation(cancellation.clone());
        let history = self.session.history.lock().await.clone();
        let previous_history_len = history.len();
        let workflow_timeout_ms = snapshot.registry.runtime_config.workflow_timeout_ms;
        let workflow = snapshot
            .registry
            .workflow
            .run(task.clone(), history, runtime_context);
        let workflow_output = if workflow_timeout_ms == 0 {
            workflow.await?
        } else {
            timeout(Duration::from_millis(workflow_timeout_ms), workflow)
                .await
                .map_err(|_| {
                    cancellation.cancel();
                    anyhow::anyhow!("workflow timed out after {workflow_timeout_ms}ms")
                })??
        };
        if cancellation.is_cancelled() {
            anyhow::bail!("turn canceled by client");
        }
        anyhow::ensure!(
            workflow_output.messages.len() >= previous_history_len,
            "workflow returned fewer messages than it received: output {}, input {}",
            workflow_output.messages.len(),
            previous_history_len
        );
        let new_messages = &workflow_output.messages[previous_history_len..];
        let memory_output = snapshot
            .registry
            .memory_policy
            .after_turn(
                MemoryPolicyInput {
                    task: &task,
                    output: &workflow_output.output,
                    new_messages,
                },
                snapshot.registry.memory.as_ref(),
            )
            .await?;
        for kind in memory_output.written_kinds {
            self.services
                .events
                .emit(
                    EventContext::new(
                        self.session.session_id,
                        self.session.thread_id,
                        Some(turn_id),
                    ),
                    Event::MemoryWritten { kind },
                )
                .await?;
        }
        let mut history = self.session.history.lock().await;
        if let Some(session_store) = &self.session.session_store {
            session_store
                .append_messages(&workflow_output.messages[previous_history_len..])
                .await?;
        }
        *history = workflow_output.messages;
        Ok(workflow_output.output)
    }

    pub async fn set_permission_mode(&self, mode: PermissionMode) {
        *self.services.permission_mode.write().await = mode;
    }

    pub async fn permission_mode(&self) -> PermissionMode {
        *self.services.permission_mode.read().await
    }

    pub async fn set_model_name(&self, model: String) {
        let model = model.trim();
        if model.is_empty() {
            return;
        }
        self.services.model_ref.write().await.model = model.to_owned();
    }

    pub async fn model_ref(&self) -> ModelRef {
        self.services.model_ref.read().await.clone()
    }

    pub async fn set_reasoning_enabled(&self, enabled: bool) {
        let mut reasoning = self.services.reasoning.write().await;
        if enabled {
            if reasoning.effort.is_none() {
                reasoning.effort = self.services.default_reasoning.effort.clone();
            }
            reasoning.summary = self.services.default_reasoning.summary;
            reasoning.budget_tokens = self.services.default_reasoning.budget_tokens;
        } else {
            reasoning.effort = None;
            reasoning.summary = false;
            reasoning.budget_tokens = None;
        }
    }

    pub async fn set_reasoning_effort(&self, effort: Option<String>) {
        self.services.reasoning.write().await.effort = effort;
    }

    pub async fn reasoning(&self) -> ReasoningConfig {
        self.services.reasoning.read().await.clone()
    }

    pub async fn tool_entries(&self) -> Vec<(ToolSource, ToolSpec)> {
        self.snapshot().await.registry.tools.entries()
    }

    pub async fn module_epoch(&self) -> ModuleEpoch {
        self.services.snapshot.read().await.epoch
    }

    async fn snapshot(&self) -> RuntimeSnapshot {
        self.services.snapshot.read().await.clone()
    }

    pub async fn reload_registry(&self, registry: BuiltinRegistry) -> Result<RuntimeReloadReport> {
        let _reload_guard = self.services.reload_lock.lock().await;
        let mut snapshot = self.services.snapshot.write().await;
        let old_epoch = snapshot.epoch;
        let new_epoch = old_epoch.next();
        let tool_names = registry
            .tools
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        *snapshot = RuntimeSnapshot::new(new_epoch, registry);
        Ok(RuntimeReloadReport {
            old_epoch: old_epoch.as_u64(),
            new_epoch: new_epoch.as_u64(),
            tool_names,
        })
    }

    pub async fn start_session(&self) -> Result<()> {
        self.ensure_session_started().await
    }

    async fn ensure_session_started(&self) -> Result<()> {
        let snapshot = self.snapshot().await;
        self.ensure_session_started_with_snapshot(&snapshot).await
    }

    async fn ensure_session_started_with_snapshot(&self, snapshot: &RuntimeSnapshot) -> Result<()> {
        let mut started = self.session.session_started.lock().await;
        if *started {
            return Ok(());
        }

        self.services
            .events
            .emit(
                EventContext::new(self.session.session_id, self.session.thread_id, None),
                Event::SessionStarted {
                    session_id: self.session.session_id,
                    cwd: self.services.cwd.clone(),
                    model: Some(snapshot.registry.model_config.model_ref()),
                    session_dir: self.session_dir().map(|path| path.to_path_buf()),
                },
            )
            .await?;
        *started = true;
        Ok(())
    }

    pub async fn render(&self, output: &AgentOutput) -> Result<String> {
        let json =
            proteus_contracts::abi_stable::std_types::RString::from(serde_json::to_string(output)?);
        let snapshot = self.snapshot().await;
        match snapshot.registry.renderer.render_json(json) {
            proteus_contracts::abi_stable::std_types::RResult::ROk(text) => Ok(text.into_string()),
            proteus_contracts::abi_stable::std_types::RResult::RErr(err) => {
                Err(anyhow::anyhow!("renderer error: {}", err.message))
            }
        }
    }

    pub async fn clear_history(&self) -> Result<()> {
        let _run_guard = self.session.run_lock.lock().await;
        self.session.history.lock().await.clear();
        if let Some(session_store) = &self.session.session_store {
            session_store.clear().await?;
        }
        Ok(())
    }

    pub async fn history_len(&self) -> usize {
        self.session.history.lock().await.len()
    }

    pub async fn history(&self) -> Vec<CanonicalMessage> {
        self.session.history.lock().await.clone()
    }

    pub fn session_dir(&self) -> Option<&std::path::Path> {
        self.session
            .session_store
            .as_ref()
            .map(|store| store.session_dir())
    }

    pub fn cwd(&self) -> &Path {
        &self.services.cwd
    }

    /// MemoryStore активной конфигурации. Используется REPL для
    /// `/remember`-команды — запись идёт напрямую в store, минуя
    /// Workflow (это не turn, а side-channel ручной записи).
    pub async fn memory(&self) -> Arc<dyn crate::contracts::MemoryStore> {
        self.snapshot().await.registry.memory.clone()
    }
}

/// Builder for `AgentRuntime`. Every slot has a sensible default
/// (headless approval, jsonl event log derived from the config, no session
/// persistence) so callers only override what they actually want to change.
pub struct AgentRuntimeBuilder {
    config: AppConfig,
    cwd: PathBuf,
    module_catalog: Option<crate::core::BuiltinModuleCatalog>,
    config_path: Option<PathBuf>,
    event_sink: Option<Arc<dyn EventSink>>,
    approval: Option<Arc<dyn ApprovalTransport>>,
    user_input: Option<Arc<dyn UserInputTransport>>,
    session_id: Option<SessionId>,
    thread_id: Option<ThreadId>,
    session_dir: Option<PathBuf>,
    resume_history: bool,
}

impl AgentRuntimeBuilder {
    pub fn new(config: AppConfig, cwd: PathBuf) -> Self {
        Self {
            config,
            cwd,
            module_catalog: None,
            config_path: None,
            event_sink: None,
            approval: None,
            user_input: None,
            session_id: None,
            thread_id: None,
            session_dir: None,
            resume_history: false,
        }
    }

    pub fn with_config_path(mut self, path: Option<&std::path::Path>) -> Self {
        self.config_path = path.map(|p| p.to_path_buf());
        self
    }

    pub fn with_module_catalog(mut self, catalog: crate::core::BuiltinModuleCatalog) -> Self {
        self.module_catalog = Some(catalog);
        self
    }

    pub fn with_event_sink(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    pub fn with_approval(mut self, approval: Arc<dyn ApprovalTransport>) -> Self {
        self.approval = Some(approval);
        self
    }

    pub fn with_user_input(mut self, user_input: Arc<dyn UserInputTransport>) -> Self {
        self.user_input = Some(user_input);
        self
    }

    pub fn with_session_ids(mut self, session_id: SessionId, thread_id: ThreadId) -> Self {
        self.session_id = Some(session_id);
        self.thread_id = Some(thread_id);
        self
    }

    pub fn resume_from_session_dir(
        mut self,
        session_dir: impl Into<PathBuf>,
        session_id: SessionId,
        thread_id: ThreadId,
    ) -> Self {
        let session_dir = session_dir.into();
        if let Ok(Some(workspace_path)) =
            crate::core::session_workspace_from_session_dir(&session_dir)
        {
            self.cwd = workspace_path;
        }
        self.session_dir = Some(session_dir);
        self.session_id = Some(session_id);
        self.thread_id = Some(thread_id);
        self.resume_history = true;
        self
    }

    pub fn build(self) -> Result<AgentRuntime> {
        let Self {
            config,
            cwd,
            module_catalog,
            config_path,
            event_sink,
            approval,
            user_input,
            session_id,
            thread_id,
            session_dir,
            resume_history,
        } = self;

        let permission_mode = config.permissions.mode;
        let registry = if let Some(catalog) = module_catalog {
            BuiltinRegistry::from_catalog(&config, cwd.clone(), catalog)?
        } else {
            BuiltinRegistry::from_config(&config, cwd.clone())?
        };
        let event_sink: Arc<dyn EventSink> = event_sink.unwrap_or_else(|| {
            let event_log_path =
                event_log_path(&config.event_log.path, config_path.as_deref(), &cwd);
            let raw: Arc<dyn EventSink> = Arc::new(JsonlEventStore::new(event_log_path));
            if config.event_log.persist_deltas {
                raw
            } else {
                // Фильтруем дельты из durable JSONL. Кастомный `event_sink`
                // (выставленный через builder) не трогаем — пользователь
                // может сам управлять что записывать, например в
                // AppServer'е где нужно и broadcast без фильтра.
                Arc::new(crate::contracts::FilteredEventSink::new(raw, |event| {
                    !crate::contracts::is_streaming_delta(event)
                }))
            }
        });
        let events = Arc::new(EventEmitter::new(event_sink));
        let approval: Arc<dyn ApprovalTransport> = Arc::new(CachedApprovalTransport::new(
            approval.unwrap_or_else(|| Arc::new(HeadlessApprovalTransport)),
        ));
        let user_input: Arc<dyn UserInputTransport> =
            user_input.unwrap_or_else(|| Arc::new(HeadlessUserInputTransport));
        let session_id = session_id.unwrap_or_else(new_session_id);
        let thread_id = thread_id.unwrap_or_else(new_thread_id);
        let session_store = if let Some(session_dir) = session_dir {
            Some(SessionStore::from_session_dir(session_dir))
        } else {
            config_path
                .as_deref()
                .map(config_store_root)
                .map(|config_dir| SessionStore::new(&config_dir, &cwd, session_id))
        };
        let history = if resume_history {
            session_store
                .as_ref()
                .map(SessionStore::load_messages)
                .transpose()?
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let session_started = resume_history && !history.is_empty();
        let model_ref = registry.model_config.model_ref();
        let reasoning = registry.model_config.reasoning.clone();
        let default_reasoning = reasoning.clone();

        Ok(AgentRuntime {
            services: RuntimeServices {
                cwd,
                snapshot: RwLock::new(RuntimeSnapshot::new(ModuleEpoch::initial(), registry)),
                reload_lock: Mutex::new(()),
                events,
                approval,
                user_input,
                permission_mode: RwLock::new(permission_mode),
                model_ref: RwLock::new(model_ref),
                reasoning: RwLock::new(reasoning),
                default_reasoning,
            },
            session: SessionState::new(
                session_id,
                thread_id,
                session_store,
                history,
                session_started,
            ),
        })
    }
}

pub fn event_log_path(configured_path: &Path, config_path: Option<&Path>, cwd: &Path) -> PathBuf {
    if configured_path.is_absolute() {
        return configured_path.to_path_buf();
    }
    config_path
        .map(config_store_root)
        .unwrap_or_else(|| cwd.to_path_buf())
        .join(configured_path)
}

pub fn config_store_root(path: &std::path::Path) -> PathBuf {
    if path.is_dir() {
        return path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf());
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if parent.file_name().and_then(|name| name.to_str()) == Some("configs")
        && let Some(root) = parent.parent()
    {
        return root.to_path_buf();
    }
    parent.to_path_buf()
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use anyhow::Result;
    use async_trait::async_trait;
    use coding_workflow::CodingPlanExecuteReviewWorkflow;
    use context_pack::SimpleContextBuilderPlugin;
    use policy_pack::AskWritePolicyPlugin;
    use proteus_contracts::{
        abi_stable::sabi_trait::TD_Opaque,
        contracts::Renderer_TO,
        plugin::{PluginApprovalPolicy_TO, PluginContextBuilder_TO, PluginWorkflow_TO},
    };
    use renderer_pack::PlainRendererPlugin;

    use super::*;
    use crate::{
        contracts::{RuntimeContext, Workflow, WorkflowOutput},
        core::{BuiltinModuleCatalog, ConfiguredToolConfig, ConfiguredToolExecutorConfig},
        domain::{AgentOutput, AgentTask, ToolSafety},
        model_standard::{CanonicalMessage, MessageRole},
    };

    fn test_catalog() -> BuiltinModuleCatalog {
        let mut catalog = BuiltinModuleCatalog::new();
        catalog
            .register_plugin_context_builder(
                "simple",
                PluginContextBuilder_TO::from_value(SimpleContextBuilderPlugin, TD_Opaque),
            )
            .expect("register test context builder");
        catalog
            .register_plugin_workflow(
                "coding.plan_execute_review",
                PluginWorkflow_TO::from_value(CodingPlanExecuteReviewWorkflow, TD_Opaque),
            )
            .expect("register test workflow");
        catalog
            .register_plugin_policy(
                "ask_write",
                PluginApprovalPolicy_TO::from_value(AskWritePolicyPlugin, TD_Opaque),
            )
            .expect("register test policy");
        catalog
            .register_plugin_renderer(
                "plain",
                Renderer_TO::from_value(PlainRendererPlugin, TD_Opaque),
            )
            .expect("register test renderer");
        catalog
    }

    struct ShortHistoryWorkflow;
    struct HangingWorkflow;
    struct DelayedWorkflow;
    struct SnapshotProbeWorkflow {
        wait_once: Arc<AtomicBool>,
        started: Arc<tokio::sync::Notify>,
        proceed: Arc<tokio::sync::Notify>,
    }

    async fn replace_workflow_for_test(runtime: &AgentRuntime, workflow: Arc<dyn Workflow>) {
        let mut snapshot = runtime.services.snapshot.write().await;
        snapshot.registry.workflow = workflow;
    }

    #[async_trait]
    impl Workflow for ShortHistoryWorkflow {
        async fn run(
            &self,
            _task: AgentTask,
            _history: Vec<CanonicalMessage>,
            _ctx: RuntimeContext,
        ) -> Result<WorkflowOutput> {
            Ok(WorkflowOutput::new(
                AgentOutput::text("bad workflow"),
                Vec::new(),
            ))
        }
    }

    #[async_trait]
    impl Workflow for HangingWorkflow {
        async fn run(
            &self,
            _task: AgentTask,
            _history: Vec<CanonicalMessage>,
            _ctx: RuntimeContext,
        ) -> Result<WorkflowOutput> {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(WorkflowOutput::new(
                AgentOutput::text("too late"),
                Vec::new(),
            ))
        }
    }

    #[async_trait]
    impl Workflow for DelayedWorkflow {
        async fn run(
            &self,
            _task: AgentTask,
            _history: Vec<CanonicalMessage>,
            _ctx: RuntimeContext,
        ) -> Result<WorkflowOutput> {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(WorkflowOutput::new(AgentOutput::text("done"), Vec::new()))
        }
    }

    #[async_trait]
    impl Workflow for SnapshotProbeWorkflow {
        async fn run(
            &self,
            _task: AgentTask,
            _history: Vec<CanonicalMessage>,
            ctx: RuntimeContext,
        ) -> Result<WorkflowOutput> {
            if self.wait_once.swap(false, Ordering::SeqCst) {
                self.started.notify_one();
                self.proceed.notified().await;
            }
            let has_late_tool = ctx.tools.spec("late_tool").is_ok();
            Ok(WorkflowOutput::new(
                AgentOutput::text(format!("has_late_tool={has_late_tool}")),
                Vec::new(),
            ))
        }
    }

    #[tokio::test]
    async fn run_errors_when_workflow_drops_existing_history() {
        let cwd = tempfile::tempdir().expect("temp dir");
        let mut config = AppConfig::default();
        config.modules.patch = "null".to_owned();
        let runtime = AgentRuntime::builder(config, cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime");

        replace_workflow_for_test(&runtime, Arc::new(ShortHistoryWorkflow)).await;
        runtime
            .session
            .history
            .lock()
            .await
            .push(CanonicalMessage::text(MessageRole::User, "previous"));

        let error = runtime
            .run("current".to_owned())
            .await
            .expect_err("short workflow history must error");

        assert!(
            error
                .to_string()
                .contains("workflow returned fewer messages than it received")
        );
    }

    #[tokio::test]
    async fn run_errors_when_workflow_timeout_is_reached() {
        let cwd = tempfile::tempdir().expect("temp dir");
        let mut config = AppConfig::default();
        config.runtime.workflow_timeout_ms = 50;
        let runtime = AgentRuntime::builder(config, cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime");
        replace_workflow_for_test(&runtime, Arc::new(HangingWorkflow)).await;

        let error = runtime
            .run("current".to_owned())
            .await
            .expect_err("hung workflow must time out");

        assert!(error.to_string().contains("workflow timed out after 50ms"));
    }

    #[tokio::test]
    async fn workflow_timeout_zero_disables_runtime_timeout() {
        let cwd = tempfile::tempdir().expect("temp dir");
        let mut config = AppConfig::default();
        config.runtime.workflow_timeout_ms = 0;
        let runtime = AgentRuntime::builder(config, cwd.path().to_path_buf())
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime");
        replace_workflow_for_test(&runtime, Arc::new(DelayedWorkflow)).await;

        let output = runtime.run("current".to_owned()).await.unwrap();

        assert_eq!(output.text, "done");
    }

    #[tokio::test]
    async fn reload_registry_publishes_new_snapshot_without_mutating_running_turn() {
        let cwd = tempfile::tempdir().expect("temp dir");
        let config = AppConfig::default();
        let runtime = Arc::new(
            AgentRuntime::builder(config, cwd.path().to_path_buf())
                .with_module_catalog(test_catalog())
                .build()
                .expect("runtime"),
        );
        let workflow = Arc::new(SnapshotProbeWorkflow {
            wait_once: Arc::new(AtomicBool::new(true)),
            started: Arc::new(tokio::sync::Notify::new()),
            proceed: Arc::new(tokio::sync::Notify::new()),
        });
        replace_workflow_for_test(&runtime, workflow.clone()).await;

        let running_runtime = runtime.clone();
        let running = tokio::spawn(async move { running_runtime.run("probe".to_owned()).await });
        workflow.started.notified().await;

        let mut next_config = AppConfig::default();
        next_config.tools.configured.push(ConfiguredToolConfig {
            name: "late_tool".to_owned(),
            description: "Appears after reload".to_owned(),
            input_schema: serde_json::json!({"type": "object"}),
            safety: ToolSafety::ReadOnly,
            timeout_ms: None,
            metadata: serde_json::Value::Null,
            executor: ConfiguredToolExecutorConfig::Process {
                command: "printf".to_owned(),
                args: vec!["ok".to_owned()],
            },
        });
        let next_registry =
            BuiltinRegistry::from_catalog(&next_config, cwd.path().to_path_buf(), test_catalog())
                .expect("next registry");
        let report = runtime
            .reload_registry(next_registry)
            .await
            .expect("reload registry");
        assert_eq!(report.old_epoch, 0);
        assert_eq!(report.new_epoch, 1);
        assert!(report.tool_names.iter().any(|name| name == "late_tool"));

        workflow.proceed.notify_one();
        let running_output = running
            .await
            .expect("running task")
            .expect("running output");
        assert_eq!(running_output.text, "has_late_tool=false");

        replace_workflow_for_test(&runtime, workflow).await;
        let next_output = runtime.run("probe after reload".to_owned()).await.unwrap();
        assert_eq!(next_output.text, "has_late_tool=true");
    }
}
