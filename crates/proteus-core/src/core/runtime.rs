use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};

use crate::{
    contracts::{ApprovalTransport, EventEmitter, EventSink, ToolSource, UserInputTransport},
    core::{
        AppConfig, AssemblyPlan, PreparedAssembly, RuntimeRegistry, SessionConfigSnapshot,
        SessionStore,
    },
    domain::{
        AgentOutput, Event, EventContext, ModelRef, PermissionMode, ReasoningConfig, SessionId,
        ThreadId, ToolSpec,
    },
    model_standard::CanonicalMessage,
};

mod builder;
mod execution_binding;
mod history;
mod paths;
mod steering;
mod turn;

pub use builder::AgentRuntimeBuilder;
pub use paths::{config_store_root, event_log_path};

pub(crate) use history::prepare_history_update;
pub(crate) use steering::{
    ReservedUserMessage, SteeringQueueReceipt, UserMessageReservation, without_root_steering,
};
use steering::{SessionSteering, SteeringFinalizationGuard};

pub struct AgentRuntime {
    services: RuntimeServices,
    session: SessionState,
}

pub(crate) struct ReservedRunCompletion {
    result: Result<AgentOutput>,
    _finalization: SteeringFinalizationGuard,
}

impl ReservedRunCompletion {
    pub(crate) fn output(&self) -> Option<&AgentOutput> {
        self.result.as_ref().ok()
    }

    pub(crate) fn error(&self) -> Option<&anyhow::Error> {
        self.result.as_ref().err()
    }

    pub(crate) fn into_result(self) -> Result<AgentOutput> {
        self.result
    }
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
    pub assembly_plan: AssemblyPlan,
    pub registry: RuntimeRegistry,
    pub config_snapshot: Option<SessionConfigSnapshot>,
}

#[derive(Clone)]
struct RuntimeExecutionState {
    runtime: RuntimeSnapshot,
    permission_mode: PermissionMode,
    model_ref: ModelRef,
    reasoning: ReasoningConfig,
}

/// Immutable execution-effective state captured once after Turn admission.
///
/// Assembly and live overrides share one lock in `RuntimeServices`, so a Turn
/// cannot combine a registry from one module epoch with settings published for
/// another runtime state.
#[derive(Clone)]
struct TurnExecutionSnapshot {
    runtime: RuntimeSnapshot,
    permission_mode: PermissionMode,
    model_ref: ModelRef,
    reasoning: ReasoningConfig,
    config_snapshot: Option<SessionConfigSnapshot>,
}

impl RuntimeSnapshot {
    pub fn new(
        epoch: ModuleEpoch,
        assembly: PreparedAssembly,
        config_snapshot: Option<SessionConfigSnapshot>,
    ) -> Self {
        let (assembly_plan, registry) = assembly.into_parts();
        Self {
            epoch,
            assembly_plan,
            registry,
            config_snapshot,
        }
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
    execution_state: RwLock<RuntimeExecutionState>,
    reload_lock: Mutex<()>,
    events: Arc<EventEmitter>,
    approval: Arc<dyn ApprovalTransport>,
    user_input: Arc<dyn UserInputTransport>,
    default_reasoning: ReasoningConfig,
}

struct SessionState {
    session_id: SessionId,
    thread_id: ThreadId,
    run_lock: Mutex<()>,
    session_started: Mutex<bool>,
    history: Mutex<Vec<CanonicalMessage>>,
    session_store: Option<SessionStore>,
    steering: Arc<SessionSteering>,
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
            steering: Arc::new(SessionSteering::default()),
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

    pub async fn set_permission_mode(&self, mode: PermissionMode) {
        self.services.execution_state.write().await.permission_mode = mode;
    }

    pub async fn permission_mode(&self) -> PermissionMode {
        self.services.execution_state.read().await.permission_mode
    }

    pub async fn set_model_name(&self, model: String) {
        let model = model.trim();
        if model.is_empty() {
            return;
        }
        self.services.execution_state.write().await.model_ref.model = model.to_owned();
    }

    /// Полная замена provider+model, например после смены `active_provider`
    /// через config builder: `reload_assembly` пересобирает model adapter, но
    /// не трогает runtime override model_ref.
    pub async fn set_model_ref(&self, model_ref: ModelRef) {
        self.services.execution_state.write().await.model_ref = model_ref;
    }

    pub async fn model_ref(&self) -> ModelRef {
        self.services.execution_state.read().await.model_ref.clone()
    }

    pub async fn set_reasoning_enabled(&self, enabled: bool) {
        let mut state = self.services.execution_state.write().await;
        let reasoning = &mut state.reasoning;
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
        let mut state = self.services.execution_state.write().await;
        let reasoning = &mut state.reasoning;
        match effort.as_deref() {
            // «none» — первоклассное значение effort: выключает рассуждения
            // целиком. Веб-клиент шлёт его вместо пары /reasoning + /effort.
            Some(value) if value.eq_ignore_ascii_case("none") => {
                reasoning.effort = None;
                reasoning.summary = false;
                reasoning.budget_tokens = None;
            }
            // Конкретный effort включает рассуждения, даже если они были
            // выключены: summary/budget возвращаются к дефолтам конфига.
            Some(value) => {
                if reasoning.effort.is_none()
                    && !reasoning.summary
                    && reasoning.budget_tokens.is_none()
                {
                    reasoning.summary = self.services.default_reasoning.summary;
                    reasoning.budget_tokens = self.services.default_reasoning.budget_tokens;
                }
                reasoning.effort = Some(value.to_owned());
            }
            // null — «auto»: явного effort нет, остальное не трогаем.
            None => reasoning.effort = None,
        }
    }

    pub async fn reasoning(&self) -> ReasoningConfig {
        self.services.execution_state.read().await.reasoning.clone()
    }

    pub async fn tool_entries(&self) -> Vec<(ToolSource, ToolSpec)> {
        self.snapshot().await.registry.tools.entries()
    }

    pub async fn module_epoch(&self) -> ModuleEpoch {
        self.services.execution_state.read().await.runtime.epoch
    }

    pub async fn assembly_plan(&self) -> AssemblyPlan {
        self.services
            .execution_state
            .read()
            .await
            .runtime
            .assembly_plan
            .clone()
    }

    pub async fn assembly_observation(
        &self,
    ) -> (ModuleEpoch, AssemblyPlan, Vec<(ToolSource, ToolSpec)>) {
        let state = self.services.execution_state.read().await;
        (
            state.runtime.epoch,
            state.runtime.assembly_plan.clone(),
            state.runtime.registry.tools.entries(),
        )
    }

    async fn snapshot(&self) -> RuntimeSnapshot {
        self.services.execution_state.read().await.runtime.clone()
    }

    async fn turn_execution_snapshot(&self) -> TurnExecutionSnapshot {
        let state = self.services.execution_state.read().await;
        let mut config_snapshot = state.runtime.config_snapshot.clone();
        if let Some(config) = &mut config_snapshot {
            config.model = state.model_ref.clone();
            config.reasoning = state.reasoning.clone();
            config.permission_mode_default = state.permission_mode;
        }
        TurnExecutionSnapshot {
            runtime: state.runtime.clone(),
            permission_mode: state.permission_mode,
            model_ref: state.model_ref.clone(),
            reasoning: state.reasoning.clone(),
            config_snapshot,
        }
    }

    pub async fn reload_assembly(
        &self,
        assembly: PreparedAssembly,
        config_snapshot: Option<SessionConfigSnapshot>,
    ) -> Result<RuntimeReloadReport> {
        self.reload_assembly_with_effective_settings(assembly, config_snapshot, None, None)
            .await
    }

    pub(crate) async fn reload_assembly_with_effective_settings(
        &self,
        assembly: PreparedAssembly,
        config_snapshot: Option<SessionConfigSnapshot>,
        model_ref: Option<ModelRef>,
        permission_mode: Option<PermissionMode>,
    ) -> Result<RuntimeReloadReport> {
        if self.session.session_store.is_some() && config_snapshot.is_none() {
            anyhow::bail!("persisted runtime reload requires a config snapshot");
        }
        let _reload_guard = self.services.reload_lock.lock().await;
        let mut state = self.services.execution_state.write().await;
        let old_epoch = state.runtime.epoch;
        let new_epoch = old_epoch.next();
        let tool_names = assembly
            .registry()
            .tools
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        state.runtime = RuntimeSnapshot::new(new_epoch, assembly, config_snapshot);
        if let Some(model_ref) = model_ref {
            state.model_ref = model_ref;
        }
        if let Some(permission_mode) = permission_mode {
            state.permission_mode = permission_mode;
        }
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
        let snapshot = self.turn_execution_snapshot().await;
        self.ensure_session_started_with_snapshot(&snapshot).await
    }

    async fn ensure_session_started_with_snapshot(
        &self,
        snapshot: &TurnExecutionSnapshot,
    ) -> Result<()> {
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
                    model: Some(snapshot.model_ref.clone()),
                    session_dir: self.session_dir().map(|path| path.to_path_buf()),
                },
            )
            .await?;
        *started = true;
        Ok(())
    }

    fn persist_config_snapshot_for_session(&self, snapshot: Option<&SessionConfigSnapshot>) {
        let (Some(session_store), Some(snapshot)) = (self.session.session_store.as_ref(), snapshot)
        else {
            return;
        };
        if let Err(error) =
            crate::core::write_config_snapshot(session_store.session_dir(), snapshot)
        {
            eprintln!("warning: failed to persist session config snapshot: {error:#}");
        }
    }

    pub async fn render(&self, output: &AgentOutput) -> Result<String> {
        let snapshot = self.snapshot().await;
        snapshot.registry.renderer.render(output)
    }

    pub async fn clear_history(&self) -> Result<()> {
        let _run_guard = self.session.run_lock.lock().await;
        self.session.steering.abort().await;
        self.session.history.lock().await.clear();
        if let Some(session_store) = &self.session.session_store {
            session_store.clear_history(self.session.thread_id).await?;
        }
        Ok(())
    }

    pub async fn history_len(&self) -> usize {
        self.session.history.lock().await.len()
    }

    pub async fn history(&self) -> Vec<CanonicalMessage> {
        self.session.history.lock().await.clone()
    }

    pub(crate) fn active_turn_id(&self) -> Option<crate::domain::TurnId> {
        self.session.steering.active_turn_id()
    }

    pub(crate) fn session_projection(&self) -> Result<Option<crate::core::JournalProjection>> {
        self.session
            .session_store
            .as_ref()
            .map(SessionStore::load_projection)
            .transpose()
    }

    pub(crate) async fn queued_user_messages(&self) -> Vec<(crate::domain::MessageId, String)> {
        self.session.steering.queued_messages().await
    }

    pub fn session_id(&self) -> crate::domain::SessionId {
        self.session.session_id
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

#[cfg(test)]
mod tests;
