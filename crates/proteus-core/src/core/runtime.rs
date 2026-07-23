use std::{
    error::Error as StdError,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use tokio::time::{Duration, timeout};

use crate::{
    contracts::{
        ApprovalTransport, CancellationToken, EventEmitter, EventSink, ToolSource,
        UserInputTransport,
    },
    core::{AppConfig, BuiltinRegistry, SessionConfigSnapshot, SessionStore},
    domain::{
        AgentOutput, AgentTask, Event, EventContext, ModelRef, PermissionMode, ReasoningConfig,
        SessionId, ThreadId, ToolSpec,
    },
    model_standard::CanonicalMessage,
};

mod builder;
mod history;
mod paths;
mod steering;

pub use builder::AgentRuntimeBuilder;
pub use paths::{config_store_root, event_log_path};

use history::prepare_history_update;
pub(crate) use steering::{
    ReservedUserMessage, SteeringQueueReceipt, UserMessageReservation, without_root_steering,
};
use steering::{
    RootTurnSettlement, SessionSteering, SteeringFinalizationGuard, SteeringModel,
    weave_deliveries_into_output,
};

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
    pub registry: BuiltinRegistry,
    pub config_snapshot: Option<SessionConfigSnapshot>,
}

impl RuntimeSnapshot {
    pub fn new(
        epoch: ModuleEpoch,
        registry: BuiltinRegistry,
        config_snapshot: Option<SessionConfigSnapshot>,
    ) -> Self {
        Self {
            epoch,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnAbort {
    Canceled,
    WorkflowTimeout { timeout_ms: u64 },
}

impl fmt::Display for TurnAbort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Canceled => formatter.write_str("turn canceled by client"),
            Self::WorkflowTimeout { timeout_ms } => {
                write!(formatter, "workflow timed out after {timeout_ms}ms")
            }
        }
    }
}

impl StdError for TurnAbort {}

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

    pub async fn run(&self, text: String) -> Result<AgentOutput> {
        self.run_with_cancellation(text, CancellationToken::new())
            .await
    }

    pub async fn run_with_cancellation(
        &self,
        text: String,
        cancellation: CancellationToken,
    ) -> Result<AgentOutput> {
        self.run_completion(text, cancellation).await?.into_result()
    }

    pub(crate) async fn run_completion(
        &self,
        text: String,
        cancellation: CancellationToken,
    ) -> Result<ReservedRunCompletion> {
        let _run_guard = self.session.run_lock.lock().await;
        let reserved = match self.reserve_user_message(text).await? {
            UserMessageReservation::Start(reserved) => reserved,
            UserMessageReservation::Queued(_) => {
                anyhow::bail!("session acquired the run lock with an active root reservation")
            }
        };
        Ok(self.run_reserved_chain(reserved, cancellation).await)
    }

    /// Atomically reserves an idle root session or appends to its bounded
    /// steering queue. App-server transports call this before spawning a turn,
    /// eliminating the race between the first and second `Send` commands.
    pub(crate) async fn reserve_user_message(
        &self,
        text: String,
    ) -> Result<UserMessageReservation> {
        let reservation = self.session.steering.reserve(text).await?;
        if let UserMessageReservation::Queued(receipt) = &reservation {
            self.services
                .events
                .emit(
                    EventContext::new(
                        self.session.session_id,
                        self.session.thread_id,
                        Some(receipt.active_turn_id),
                    ),
                    Event::SteeringQueued {
                        message_id: receipt.message_id,
                        text: receipt.text.clone(),
                        queued_count: receipt.queued_count,
                    },
                )
                .await?;
        }
        Ok(reservation)
    }

    #[cfg(test)]
    pub(crate) async fn run_reserved_with_cancellation(
        &self,
        reserved: ReservedUserMessage,
        cancellation: CancellationToken,
    ) -> Result<AgentOutput> {
        self.run_reserved_completion(reserved, cancellation)
            .await?
            .into_result()
    }

    pub(crate) async fn run_reserved_completion(
        &self,
        reserved: ReservedUserMessage,
        cancellation: CancellationToken,
    ) -> Result<ReservedRunCompletion> {
        let _run_guard = self.session.run_lock.lock().await;
        self.session
            .steering
            .validate_reservation(reserved.turn_id)
            .await?;
        Ok(self.run_reserved_chain(reserved, cancellation).await)
    }

    async fn run_reserved_chain(
        &self,
        mut reserved: ReservedUserMessage,
        cancellation: CancellationToken,
    ) -> ReservedRunCompletion {
        let mut reservation_guard = self.session.steering.run_guard();
        let settled = async {
            loop {
                let turn_id = reserved.turn_id;
                let output = self.run_one_turn(reserved, cancellation.clone()).await?;
                if cancellation.is_cancelled() {
                    anyhow::bail!("turn canceled by client");
                }
                match self
                    .session
                    .steering
                    .settle_and_take_followup(turn_id)
                    .await?
                {
                    RootTurnSettlement::FollowUp(followup) => reserved = followup,
                    RootTurnSettlement::Complete(finalization) => {
                        return Ok((output, finalization));
                    }
                }
            }
        }
        .await;

        let (result, finalization) = match settled {
            Ok((output, finalization)) => (Ok(output), finalization),
            Err(error) => {
                let finalization = self.session.steering.finalization_guard().await;
                (Err(error), finalization)
            }
        };
        reservation_guard.disarm();
        ReservedRunCompletion {
            result,
            _finalization: finalization,
        }
    }

    async fn run_one_turn(
        &self,
        reserved: ReservedUserMessage,
        cancellation: CancellationToken,
    ) -> Result<AgentOutput> {
        let snapshot = self.snapshot().await;
        self.ensure_session_started_with_snapshot(&snapshot).await?;
        let turn_id = reserved.turn_id;
        let task = AgentTask::new(reserved.text.clone(), self.services.cwd.clone());
        if cancellation.is_cancelled() {
            return Err(TurnAbort::Canceled.into());
        }
        let config_snapshot = self.turn_config_snapshot(&snapshot).await;
        if let Some(session_store) = &self.session.session_store {
            let base_history_revision = session_store.load_projection()?.history_revision;
            session_store
                .append_journal_entry(
                    self.session.thread_id,
                    Some(turn_id),
                    crate::core::JournalEntry::TurnOpened(crate::core::TurnOpened {
                        task: task.clone(),
                        base_history_revision,
                        module_epoch: snapshot.epoch.as_u64(),
                        config_snapshot: serde_json::to_value(config_snapshot.as_ref())?,
                    }),
                )
                .await?;
        }
        let result = self
            .run_opened_turn(
                reserved,
                cancellation.clone(),
                snapshot,
                config_snapshot,
                task,
            )
            .await;
        let settlement = match &result {
            Ok(output) => crate::core::TurnSettled {
                status: crate::core::TurnSettlementStatus::Success,
                output: Some(output.clone()),
                error: None,
            },
            Err(error) => {
                let message = format!("{error:#}");
                let status = turn_settlement_status(error, cancellation.is_cancelled());
                crate::core::TurnSettled {
                    status,
                    output: None,
                    error: Some(message),
                }
            }
        };
        if let Some(session_store) = &self.session.session_store
            && let Err(settlement_error) = session_store
                .append_journal_entry(
                    self.session.thread_id,
                    Some(turn_id),
                    crate::core::JournalEntry::TurnSettled(settlement),
                )
                .await
        {
            return match result {
                Ok(_) => Err(settlement_error
                    .context("turn completed but its canonical settlement could not be persisted")),
                Err(turn_error) => Err(anyhow::anyhow!(
                    "{turn_error:#}; additionally failed to persist turn settlement: {settlement_error:#}"
                )),
            };
        }
        result
    }

    async fn run_opened_turn(
        &self,
        reserved: ReservedUserMessage,
        cancellation: CancellationToken,
        snapshot: RuntimeSnapshot,
        config_snapshot: Option<SessionConfigSnapshot>,
        task: AgentTask,
    ) -> Result<AgentOutput> {
        let turn_id = reserved.turn_id;
        let user_message = reserved.message;
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
        let history = self
            .persist_current_user_message(turn_id, &user_message, config_snapshot.as_ref())
            .await?;
        if let Some(kind) = reserved.delivery {
            self.services
                .events
                .emit(
                    EventContext::new(
                        self.session.session_id,
                        self.session.thread_id,
                        Some(turn_id),
                    ),
                    Event::SteeringDelivered {
                        message_id: user_message.id,
                        text: task.text.clone(),
                        kind,
                        queued_count: self
                            .session
                            .steering
                            .queued_count_handle()
                            .load(std::sync::atomic::Ordering::Acquire),
                    },
                )
                .await?;
        }
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
                session_store: self.session.session_store.clone(),
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
        runtime_context.queued_user_messages = self.session.steering.queued_count_handle();
        if let Some(session_store) = &self.session.session_store {
            runtime_context.execution_recorder = Arc::new(
                crate::core::SessionExecutionRecorder::new(session_store.clone()),
            );
        }
        let steering_model = SteeringModel::new(
            runtime_context.model.clone(),
            self.session.steering.clone(),
            self.services.events.clone(),
            self.session.session_id,
            self.session.thread_id,
            turn_id,
        );
        runtime_context.model = Arc::new(steering_model.clone());
        let runtime_context = runtime_context.with_cancellation(cancellation.clone());
        let workflow_timeout_ms = snapshot.registry.runtime_config.workflow_timeout_ms;
        let workflow =
            snapshot
                .registry
                .workflow
                .run(task.clone(), history.clone(), runtime_context);
        let workflow_result = if workflow_timeout_ms == 0 {
            workflow.await
        } else {
            match timeout(Duration::from_millis(workflow_timeout_ms), workflow).await {
                Ok(result) => result,
                Err(_) => {
                    cancellation.cancel();
                    Err(TurnAbort::WorkflowTimeout {
                        timeout_ms: workflow_timeout_ms,
                    }
                    .into())
                }
            }
        };
        let delivery_records = steering_model.delivery_records().await;
        let mut workflow_output = match workflow_result {
            Ok(output) => output,
            Err(error) => {
                return self
                    .fail_turn_preserving_steering(turn_id, error, &delivery_records)
                    .await;
            }
        };
        if cancellation.is_cancelled() {
            return self
                .fail_turn_preserving_steering(
                    turn_id,
                    TurnAbort::Canceled.into(),
                    &delivery_records,
                )
                .await;
        }
        let runtime_user_messages =
            match weave_deliveries_into_output(&mut workflow_output, &delivery_records) {
                Ok(messages) => messages,
                Err(error) => {
                    return self
                        .fail_turn_preserving_steering(turn_id, error, &delivery_records)
                        .await;
                }
            };
        let history_compacted = workflow_output
            .compactions
            .iter()
            .any(|report| report.changed);
        let history_update = match prepare_history_update(
            &history,
            &user_message,
            &workflow_output.new_messages,
            workflow_output.history_replacement.as_deref(),
            history_compacted,
            &runtime_user_messages,
        ) {
            Ok(update) => update,
            Err(error) => {
                return self
                    .fail_turn_preserving_steering(turn_id, error, &delivery_records)
                    .await;
            }
        };
        let mut history = self.session.history.lock().await;
        if let Some(session_store) = &self.session.session_store {
            if history_update.replace {
                session_store
                    .replace_history(
                        self.session.thread_id,
                        Some(turn_id),
                        &history_update.final_messages,
                        workflow_output
                            .compactions
                            .iter()
                            .rev()
                            .find(|report| report.changed)
                            .cloned(),
                    )
                    .await?;
            } else {
                session_store
                    .append_history(
                        self.session.thread_id,
                        Some(turn_id),
                        &workflow_output.new_messages,
                    )
                    .await?;
            }
        }
        *history = history_update.final_messages;
        Ok(workflow_output.output)
    }

    async fn fail_turn_preserving_steering(
        &self,
        turn_id: crate::domain::TurnId,
        turn_error: anyhow::Error,
        deliveries: &[steering::SteeringDeliveryRecord],
    ) -> Result<AgentOutput> {
        if let Err(persist_error) = self
            .persist_failed_steering_messages(turn_id, deliveries)
            .await
        {
            return Err(anyhow::anyhow!(
                "{turn_error:#}; additionally failed to persist delivered steering messages: {persist_error:#}"
            ));
        }
        Err(turn_error)
    }

    async fn persist_failed_steering_messages(
        &self,
        turn_id: crate::domain::TurnId,
        deliveries: &[steering::SteeringDeliveryRecord],
    ) -> Result<()> {
        let mut history = self.session.history.lock().await;
        let messages = deliveries
            .iter()
            .map(|delivery| delivery.message.clone())
            .filter(|message| !history.iter().any(|stored| stored.id == message.id))
            .collect::<Vec<_>>();
        if messages.is_empty() {
            return Ok(());
        }
        if let Some(session_store) = &self.session.session_store {
            session_store
                .append_history(self.session.thread_id, Some(turn_id), &messages)
                .await?;
        }
        history.extend(messages);
        Ok(())
    }

    async fn persist_current_user_message(
        &self,
        turn_id: crate::domain::TurnId,
        user_message: &CanonicalMessage,
        config_snapshot: Option<&SessionConfigSnapshot>,
    ) -> Result<Vec<CanonicalMessage>> {
        let mut history = self.session.history.lock().await;
        if let Some(session_store) = &self.session.session_store {
            session_store
                .append_history(
                    self.session.thread_id,
                    Some(turn_id),
                    std::slice::from_ref(user_message),
                )
                .await?;
            self.persist_config_snapshot_for_session(config_snapshot);
        }
        history.push(user_message.clone());
        Ok(history.clone())
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

    /// Полная замена provider+model, например после смены `active_provider`
    /// через config builder: `reload_registry` пересобирает model adapter, но
    /// не трогает runtime override model_ref.
    pub async fn set_model_ref(&self, model_ref: ModelRef) {
        *self.services.model_ref.write().await = model_ref;
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
        let mut reasoning = self.services.reasoning.write().await;
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

    pub async fn reload_registry(
        &self,
        registry: BuiltinRegistry,
        config_snapshot: Option<SessionConfigSnapshot>,
    ) -> Result<RuntimeReloadReport> {
        if self.session.session_store.is_some() && config_snapshot.is_none() {
            anyhow::bail!("persisted runtime reload requires a config snapshot");
        }
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
        *snapshot = RuntimeSnapshot::new(new_epoch, registry, config_snapshot);
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

    async fn turn_config_snapshot(
        &self,
        snapshot: &RuntimeSnapshot,
    ) -> Option<SessionConfigSnapshot> {
        let mut config = snapshot.config_snapshot.clone()?;
        config.model = self.services.model_ref.read().await.clone();
        config.reasoning = self.services.reasoning.read().await.clone();
        config.permission_mode_default = *self.services.permission_mode.read().await;
        Some(config)
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

fn turn_settlement_status(
    error: &anyhow::Error,
    cancellation_is_set: bool,
) -> crate::core::TurnSettlementStatus {
    for cause in error.chain() {
        match cause.downcast_ref::<TurnAbort>() {
            Some(TurnAbort::WorkflowTimeout { .. }) => {
                return crate::core::TurnSettlementStatus::Timeout;
            }
            Some(TurnAbort::Canceled) => {
                return crate::core::TurnSettlementStatus::Canceled;
            }
            None => {}
        }
    }
    if cancellation_is_set {
        crate::core::TurnSettlementStatus::Canceled
    } else {
        crate::core::TurnSettlementStatus::Error
    }
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
        plugin::{PluginApprovalPolicy_TO, PluginContextBuilder_TO, PluginWorkflow_TO},
    };

    use super::*;
    use crate::{
        contracts::{RuntimeContext, Workflow, WorkflowOutput},
        core::{BuiltinModuleCatalog, ConfiguredToolConfig, ConfiguredToolExecutorConfig},
        domain::{AgentOutput, AgentTask, HistoryCompactionReport, ToolSafety},
        model_standard::{CanonicalMessage, CanonicalModelRequest, MessageRole},
    };

    mod steering_integration;

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
    }

    fn message_text_for_test(message: &CanonicalMessage) -> String {
        message
            .parts
            .iter()
            .filter_map(|part| match &part.payload {
                crate::model_standard::ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn settlement_status_uses_typed_runtime_causes_not_error_text() {
        let misleading = anyhow::anyhow!("provider rejected field named cancellation_timeout");
        assert_eq!(
            turn_settlement_status(&misleading, false),
            crate::core::TurnSettlementStatus::Error
        );

        let timeout = anyhow::Error::new(TurnAbort::WorkflowTimeout { timeout_ms: 10 });
        assert_eq!(
            turn_settlement_status(&timeout, true),
            crate::core::TurnSettlementStatus::Timeout
        );

        assert_eq!(
            turn_settlement_status(&misleading, true),
            crate::core::TurnSettlementStatus::Canceled
        );
    }

    fn successful_messages(
        history: Vec<CanonicalMessage>,
        task: AgentTask,
        answer: impl Into<String>,
    ) -> WorkflowOutput {
        let current_user = history.last().expect("persisted current user message");
        assert_eq!(current_user.role, MessageRole::User);
        assert_eq!(message_text_for_test(current_user), task.text);
        let assistant = CanonicalMessage::text(MessageRole::Assistant, answer.into());
        WorkflowOutput::new(AgentOutput::text("done"), vec![assistant])
    }

    struct ShortHistoryWorkflow;
    struct CompactingWorkflow;
    struct HangingWorkflow;
    struct DelayedWorkflow;
    struct ModelCallingWorkflow;
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
    impl Workflow for CompactingWorkflow {
        async fn run(
            &self,
            task: AgentTask,
            history: Vec<CanonicalMessage>,
            _ctx: RuntimeContext,
        ) -> Result<WorkflowOutput> {
            assert_eq!(history.len(), 3);
            let summary = CanonicalMessage::text(MessageRole::User, "compacted summary");
            let current = history.last().expect("current user").clone();
            assert_eq!(message_text_for_test(&current), task.text);
            let answer = CanonicalMessage::text(MessageRole::Assistant, "done after compact");
            let mut report = HistoryCompactionReport::unchanged(
                history.len(),
                Some("test_compaction".to_owned()),
            );
            report.changed = true;
            report.output_messages = 3;
            report.original_token_estimate = Some(500);
            report.output_token_estimate = Some(50);
            report.trigger_tokens = Some(100);
            report.summary_source = Some("test".to_owned());
            report.summary = Some("compacted summary".to_owned());
            report.metadata = serde_json::json!({"test": true});
            Ok(WorkflowOutput::new(AgentOutput::text("done"), vec![answer])
                .with_history_replacement(vec![summary, current])
                .with_compactions(vec![report]))
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
            task: AgentTask,
            history: Vec<CanonicalMessage>,
            _ctx: RuntimeContext,
        ) -> Result<WorkflowOutput> {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(successful_messages(history, task, "done"))
        }
    }

    #[async_trait]
    impl Workflow for ModelCallingWorkflow {
        async fn run(
            &self,
            task: AgentTask,
            history: Vec<CanonicalMessage>,
            ctx: RuntimeContext,
        ) -> Result<WorkflowOutput> {
            let current_user = history.last().expect("persisted current user message");
            assert_eq!(message_text_for_test(current_user), task.text);
            let request = CanonicalModelRequest::new(ctx.model_ref.clone(), history)
                .with_instructions(ctx.instructions.clone())
                .with_tools(ctx.tools.specs())
                .with_reasoning(ctx.reasoning.clone());
            let response = ctx.model.complete(request).await?;
            Ok(WorkflowOutput::new(
                AgentOutput::text("done"),
                vec![response.message],
            ))
        }
    }

    #[async_trait]
    impl Workflow for SnapshotProbeWorkflow {
        async fn run(
            &self,
            task: AgentTask,
            history: Vec<CanonicalMessage>,
            ctx: RuntimeContext,
        ) -> Result<WorkflowOutput> {
            if self.wait_once.swap(false, Ordering::SeqCst) {
                self.started.notify_one();
                self.proceed.notified().await;
            }
            let has_late_tool = ctx.tools.spec("late_tool").is_ok();
            let output = AgentOutput::text(format!("has_late_tool={has_late_tool}"));
            let current_user = history.last().expect("persisted current user message");
            assert_eq!(message_text_for_test(current_user), task.text);
            let assistant = CanonicalMessage::text(MessageRole::Assistant, output.text.clone());
            Ok(WorkflowOutput::new(output, vec![assistant]))
        }
    }

    #[tokio::test]
    async fn run_errors_when_workflow_returns_no_turn_messages() {
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
            .expect_err("empty workflow turn must error");

        assert!(
            error
                .to_string()
                .contains("workflow returned no new persistent turn messages")
        );
    }

    #[tokio::test]
    async fn failed_turn_keeps_user_message_in_runtime_and_session_store() {
        let config_root = tempfile::tempdir().expect("config root");
        let workspace = tempfile::tempdir().expect("workspace");
        let config_path = config_root.path().join("configs").join("config.toml");
        let mut config = AppConfig::default();
        config.modules.patch = "null".to_owned();
        let runtime = AgentRuntime::builder(config, workspace.path().to_path_buf())
            .with_config_path(Some(&config_path))
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime");
        replace_workflow_for_test(&runtime, Arc::new(ShortHistoryWorkflow)).await;

        let error = runtime
            .run("current request".to_owned())
            .await
            .expect_err("bad workflow must fail");

        assert!(
            error
                .to_string()
                .contains("workflow returned no new persistent turn messages")
        );
        let history = runtime.history().await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].role, MessageRole::User);
        assert_eq!(message_text_for_test(&history[0]), "current request");
        let stored = runtime
            .session
            .session_store
            .as_ref()
            .expect("session store")
            .load_messages()
            .expect("load messages");
        assert_eq!(stored, history);
    }

    #[tokio::test]
    async fn compaction_replaces_runtime_and_session_history() {
        let config_root = tempfile::tempdir().expect("config root");
        let workspace = tempfile::tempdir().expect("workspace");
        let config_path = config_root.path().join("configs").join("config.toml");
        let mut config = AppConfig::default();
        config.modules.patch = "null".to_owned();
        let runtime = AgentRuntime::builder(config, workspace.path().to_path_buf())
            .with_config_path(Some(&config_path))
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime");

        replace_workflow_for_test(&runtime, Arc::new(CompactingWorkflow)).await;
        {
            let mut history = runtime.session.history.lock().await;
            history.push(CanonicalMessage::text(MessageRole::User, "old request"));
            history.push(CanonicalMessage::text(MessageRole::Assistant, "old answer"));
        }
        let seed_history = runtime.session.history.lock().await.clone();
        runtime
            .session
            .session_store
            .as_ref()
            .expect("session store")
            .append_history(runtime.session.thread_id, None, &seed_history)
            .await
            .expect("seed session store");

        runtime.run("current request".to_owned()).await.unwrap();

        let history = runtime.history().await;
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].role, MessageRole::User);
        assert!(message_text_for_test(&history[0]).contains("compacted summary"));
        assert!(message_text_for_test(&history[1]).contains("current request"));
        assert!(message_text_for_test(&history[2]).contains("done after compact"));

        let stored = runtime
            .session
            .session_store
            .as_ref()
            .expect("session store")
            .load_messages()
            .expect("load replaced messages");
        assert_eq!(stored, history);
    }

    #[tokio::test]
    async fn model_exchange_is_recorded_in_session_journal() {
        let config_root = tempfile::tempdir().expect("config root");
        let workspace = tempfile::tempdir().expect("workspace");
        let config_path = config_root.path().join("configs").join("config.toml");
        let mut config = AppConfig::default();
        config.modules.patch = "null".to_owned();
        let runtime = AgentRuntime::builder(config, workspace.path().to_path_buf())
            .with_config_path(Some(&config_path))
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime");
        replace_workflow_for_test(&runtime, Arc::new(ModelCallingWorkflow)).await;

        runtime.run("record exchange".to_owned()).await.unwrap();

        let records = runtime
            .session
            .session_store
            .as_ref()
            .expect("session store")
            .load_records()
            .expect("journal records");
        assert!(records.iter().any(|record| matches!(
            record.entry,
            crate::core::JournalEntry::ModelRequestRecorded(_)
        )));
        assert!(records.iter().any(|record| matches!(
            record.entry,
            crate::core::JournalEntry::ModelResponseRecorded(_)
        )));
    }

    #[tokio::test]
    async fn runtime_writes_config_snapshot_when_session_is_persisted() {
        let config_root = tempfile::tempdir().expect("config root");
        let workspace = tempfile::tempdir().expect("workspace");
        let config_path = config_root.path().join("configs").join("config.toml");
        let mut config = AppConfig::default();
        config.profile.name = "snapshot-profile".to_owned();
        config.permissions.mode = PermissionMode::Auto;
        config.modules.workflow = "coding.plan_execute_review".to_owned();
        config.modules.context = "simple".to_owned();
        config.modules.policy = "ask_write".to_owned();
        config.modules.compactor = "none".to_owned();
        config.modules.tool_exposure = "all_visible".to_owned();
        let runtime = AgentRuntime::builder(config, workspace.path().to_path_buf())
            .with_config_path(Some(&config_path))
            .with_module_catalog(test_catalog())
            .build()
            .expect("runtime");
        runtime.start_session().await.expect("start session");
        assert!(!runtime.session_dir().expect("session dir").exists());
        replace_workflow_for_test(&runtime, Arc::new(DelayedWorkflow)).await;
        runtime.run("persist session".to_owned()).await.unwrap();

        let snapshot_path = runtime
            .session_dir()
            .expect("session dir")
            .join(crate::core::CONFIG_SNAPSHOT_FILE);
        let value: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(snapshot_path).expect("read config snapshot"),
        )
        .expect("config snapshot json");

        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["active_provider"], "fake");
        assert_eq!(value["profile_name"], "snapshot-profile");
        assert_eq!(value["modules"]["workflow"], "coding.plan_execute_review");
        assert_eq!(value["modules"]["context"], "simple");
        assert_eq!(value["modules"]["policy"], "ask_write");
        assert_eq!(value["modules"]["compactor"], "none");
        assert_eq!(value["modules"]["tool_exposure"], "all_visible");
        assert_eq!(value["permission_mode_default"], "auto");
        assert!(value["tools"].as_array().is_some());

        let mut reloaded_config = AppConfig::default();
        reloaded_config.profile.name = "reloaded-profile".to_owned();
        reloaded_config.modules.patch = "null".to_owned();
        let mut reloaded_registry = BuiltinRegistry::from_catalog(
            &reloaded_config,
            workspace.path().to_path_buf(),
            test_catalog(),
        )
        .expect("reloaded registry");
        reloaded_registry.workflow = Arc::new(DelayedWorkflow);
        let reloaded_snapshot = SessionConfigSnapshot::from_runtime_config(
            &reloaded_config,
            &reloaded_registry,
            PermissionMode::Normal,
        );
        runtime
            .reload_registry(reloaded_registry, Some(reloaded_snapshot))
            .await
            .expect("reload registry");
        runtime
            .set_model_name("runtime-model-override".to_owned())
            .await;
        runtime.set_reasoning_effort(Some("high".to_owned())).await;
        runtime.set_permission_mode(PermissionMode::Plan).await;
        runtime.run("after reload".to_owned()).await.unwrap();

        let records = runtime
            .session
            .session_store
            .as_ref()
            .expect("session store")
            .load_records()
            .expect("journal records");
        let (opened, module_epoch) = records
            .iter()
            .rev()
            .find_map(|record| match &record.entry {
                crate::core::JournalEntry::TurnOpened(opened) => {
                    Some((opened, opened.module_epoch))
                }
                _ => None,
            })
            .expect("latest turn_opened");
        let turn_snapshot: SessionConfigSnapshot =
            serde_json::from_value(opened.config_snapshot.clone()).expect("turn config snapshot");
        assert_eq!(module_epoch, 1);
        assert_eq!(turn_snapshot.profile_name, "reloaded-profile");
        assert_eq!(turn_snapshot.model.model, "runtime-model-override");
        assert_eq!(turn_snapshot.reasoning.effort.as_deref(), Some("high"));
        assert_eq!(turn_snapshot.permission_mode_default, PermissionMode::Plan);
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
        let next_registry =
            BuiltinRegistry::from_catalog(&next_config, cwd.path().to_path_buf(), test_catalog())
                .expect("next registry");
        let report = runtime
            .reload_registry(next_registry, None)
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
