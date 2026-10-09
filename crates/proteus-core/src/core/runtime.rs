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
mod checkpoint;
mod conversation;
mod execution;
mod execution_binding;
mod failed_history;
mod history;
mod hooks;
mod images;
mod paths;
mod reload;
mod settings;
mod steering;
mod turn;

pub use builder::AgentRuntimeBuilder;
pub use paths::{config_store_root, event_log_path};

use execution::ExecutionAdmissionSnapshot;
pub(crate) use history::{prepare_failed_history_update, prepare_history_update};
pub(crate) use steering::{
    QueuedMessagesSnapshot, SteeringQueueReceipt, UserMessageReservation, without_root_steering,
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
    model_ref: Option<ModelRef>,
    reasoning: ReasoningConfig,
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
    images: images::ImageStore,
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
    run_lock: Arc<Mutex<()>>,
    session_started: Mutex<bool>,
    history: Arc<Mutex<Vec<CanonicalMessage>>>,
    history_turn: Arc<Mutex<Option<crate::domain::TurnId>>>,
    model_context: Arc<Mutex<super::model_context::ModelContextState>>,
    interrupted_turns: Arc<Mutex<Vec<crate::contracts::WorkflowHistoryInterruption>>>,
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
            run_lock: Default::default(),
            session_started: Mutex::new(session_started),
            history: Arc::new(Mutex::new(history)),
            history_turn: Default::default(),
            model_context: Default::default(),
            interrupted_turns: Default::default(),
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
        let _guard = self.services.reload_lock.lock().await;
        self.services.execution_state.write().await.permission_mode = mode;
    }

    pub async fn permission_mode(&self) -> PermissionMode {
        self.services.execution_state.read().await.permission_mode
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

    pub async fn start_session(&self) -> Result<()> {
        self.ensure_session_started().await
    }

    async fn ensure_session_started(&self) -> Result<()> {
        let snapshot = self.capture_execution_snapshot().await;
        self.ensure_session_started_with_snapshot(&snapshot).await
    }

    async fn ensure_session_started_with_snapshot(
        &self,
        snapshot: &ExecutionAdmissionSnapshot,
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
                    model: snapshot.model_ref.clone(),
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

    pub async fn clear_history(&self) -> Result<()> {
        let guard = self.session.run_lock.clone().lock_owned().await;
        let store = self.session.session_store.clone();
        let thread_id = self.session.thread_id;
        let history = self.session.history.clone();
        let history_turn = self.session.history_turn.clone();
        let model_context = self.session.model_context.clone();
        let interrupted_turns = self.session.interrupted_turns.clone();
        let steering = self.session.steering.clone();
        // The operation owns durable and live settlement together. A caller
        // dropping its wait cannot expose old warm state after an admitted clear.
        tokio::spawn(async move {
            let _run_guard = guard;
            if let Some(store) = store {
                if let Err(error) = store.clear_history(thread_id).await {
                    let projection = store.load_projection().map_err(|recovery| {
                        anyhow::anyhow!("{error:#}; additionally failed to recover history after clear: {recovery:#}")
                    })?;
                    history::refresh_committed_history(&mut *history.lock().await, projection.history)?;
                    *model_context.lock().await =
                        crate::core::model_context::ModelContextState::from_records(&projection.records, None);
                    *history_turn.lock().await = projection.records.iter().rev().find_map(|record| {
                        matches!(&record.entry, crate::core::JournalEntry::HistoryMutated(_)).then_some(record.turn_id)
                    }).flatten();
                    *interrupted_turns.lock().await = projection.interrupted_turns;
                    return Err(error);
                }
            }
            steering.abort().await;
            history.lock().await.clear();
            *history_turn.lock().await = None;
            interrupted_turns.lock().await.clear();
            *model_context.lock().await = Default::default();
            Ok(())
        }).await.map_err(|error| anyhow::anyhow!("clear history task failed: {error}"))?
    }

    pub async fn history_len(&self) -> usize {
        self.session.history.lock().await.len()
    }

    pub async fn history(&self) -> Vec<CanonicalMessage> {
        self.session.history.lock().await.clone()
    }

    pub(crate) fn session_projection(&self) -> Result<Option<crate::core::JournalProjection>> {
        self.session
            .session_store
            .as_ref()
            .map(SessionStore::load_projection)
            .transpose()
    }

    pub(crate) fn subscribe_queued_user_messages(
        &self,
    ) -> tokio::sync::watch::Receiver<QueuedMessagesSnapshot> {
        self.session.steering.subscribe_queue()
    }

    #[cfg(test)]
    pub(crate) async fn queued_user_messages(&self) -> Vec<(crate::domain::MessageId, String)> {
        self.session.steering.queued_messages().await
    }

    pub fn session_id(&self) -> crate::domain::SessionId {
        self.session.session_id
    }

    pub async fn usage_snapshot(&self) -> Result<Option<crate::domain::SessionUsageSnapshot>> {
        match &self.session.session_store {
            Some(store) => store.usage_snapshot().await.map(Some),
            None => Ok(None),
        }
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
}

#[cfg(test)]
mod tests;

pub(crate) use turn::turn_settlement_status;
