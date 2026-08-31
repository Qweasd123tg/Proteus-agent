use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock, broadcast};

use crate::{
    contracts::{
        ApprovalCacheScope, ApprovalResponse, CancellationToken, EventSink, FilteredEventSink,
        UserInputResponse, is_streaming_delta,
    },
    core::{
        AgentRuntime, AppConfig, AssemblyPlan, BroadcastEventSink, ChannelApprovalTransport,
        ChannelUserInputTransport, FanoutEventSink, JsonlEventStore, ModuleCatalog,
        PreparedAssembly, ReservedRunCompletion, ReservedUserMessage, RuntimeReloadReport,
        SessionConfigSnapshot, SessionStore, TopologyBuildInput, TopologySnapshot,
        UserMessageReservation, build_topology_snapshot, config_store_root,
        delete_workspace_session, list_session_summaries, list_workspace_session_summaries,
        normalize_session_dir_path,
    },
    domain::{AgentOutput, EventEnvelope, PermissionMode, SessionId, new_thread_id},
};

mod approval_preview;
mod approvals;
mod config_builder;
mod config_summary;
mod context_map;
pub mod http;
mod path_utils;
pub mod stdio;
mod transcript;
mod turn_progress;
mod user_inputs;

pub use config_builder::{
    ConfigBuilderModule, ConfigBuilderModuleSelection, ConfigBuilderProvider, ConfigBuilderSlot,
    ConfigBuilderSnapshot, ConfigBuilderTool, ConfigBuilderWarning,
};
use config_builder::{
    config_builder_snapshot_from_topology, config_builder_target_path, persist_config_builder,
    read_toml_document_or_empty, set_module_slot, validate_config_builder_modules,
    validate_config_builder_provider, validate_module_config_toml,
};
use config_summary::{
    config_files, configured_model_options, configured_reasoning_effort_options, module_summary,
    render_config_summary,
};
use context_map::{ContextMapInput, build_context_map_snapshot};
use path_utils::paths_equal;
pub(crate) use transcript::journal_transcript_messages;
use transcript::transcript_messages;
pub use transcript::{AppTranscriptMessage, AppTranscriptSubagent, AppTranscriptTool};
use turn_progress::TurnProgress;

// Публичный app-server façade экспортирует canonical wire types из contracts;
// их определения не дублируются в core.
pub use proteus_contracts::app_protocol::{
    AppApprovalId, AppApprovalPreview, AppApprovalRequest, AppContextBuildSnapshot,
    AppContextCompactionSnapshot, AppContextHistorySummary, AppContextMapSnapshot,
    AppContextToolSummary, AppContextUsageCategory, AppContextUsageSnapshot, AppPendingRequests,
    AppQueuedUserMessage, AppServerEvent, AppSessionActivity, AppSessionSummary,
    AppUserInputRequestId, StdioOutput, StdioRequest,
};

use approvals::PendingApprovalResponders;
use user_inputs::{PendingUserInputResponders, resolve_pending_user_inputs_empty};

#[derive(Clone)]
pub struct AppServerHandle {
    runtime: Arc<AgentRuntime>,
    config: Arc<RwLock<AppConfig>>,
    config_path: Option<PathBuf>,
    cwd: PathBuf,
    events: broadcast::Sender<AppServerEvent>,
    pending_approvals: PendingApprovalResponders,
    pending_user_inputs: PendingUserInputResponders,
    turn_progress: Arc<Mutex<TurnProgress>>,
}

impl AppServerHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<AppServerEvent> {
        self.events.subscribe()
    }

    pub fn cwd_path(&self) -> &Path {
        &self.cwd
    }

    pub fn session_dir_path(&self) -> Option<PathBuf> {
        self.runtime.session_dir().map(Path::to_path_buf)
    }

    pub fn session_id(&self) -> SessionId {
        self.runtime.session_id()
    }

    pub async fn start_session(&self) -> Result<()> {
        self.runtime.start_session().await
    }

    pub async fn send_user_message(&self, text: String) -> Result<AgentOutput> {
        self.send_user_message_with_cancellation(text, CancellationToken::new())
            .await
    }

    pub async fn send_user_message_with_cancellation(
        &self,
        text: String,
        cancellation: CancellationToken,
    ) -> Result<AgentOutput> {
        let _ = self
            .events
            .send(AppServerEvent::UserMessageSubmitted { text: text.clone() });
        let completion = self.runtime.run_completion(text, cancellation).await;
        // Ход завершён: его сообщения уже в history (или потеряны при ошибке),
        // прогресс очищаем до эмита TurnOutput — чтобы /history, вызванный по
        // TurnOutput, не отдал текст хода дважды. Отмена и таймауты тоже
        // проходят здесь, у форвардера событий такой гарантии нет.
        self.turn_progress.lock().await.finish_parent_turn();
        self.publish_turn_completion(completion)
    }

    pub(crate) async fn reserve_user_message(
        &self,
        text: String,
    ) -> Result<UserMessageReservation> {
        let reservation = self.runtime.reserve_user_message(text.clone()).await?;
        if matches!(reservation, UserMessageReservation::Start(_)) {
            let _ = self
                .events
                .send(AppServerEvent::UserMessageSubmitted { text });
        }
        Ok(reservation)
    }

    pub(crate) async fn run_reserved_user_message(
        &self,
        reserved: ReservedUserMessage,
        cancellation: CancellationToken,
    ) -> Result<AgentOutput> {
        let completion = self
            .runtime
            .run_reserved_completion(reserved, cancellation)
            .await;
        self.turn_progress.lock().await.finish_parent_turn();
        self.publish_turn_completion(completion)
    }

    fn publish_turn_completion(
        &self,
        completion: Result<ReservedRunCompletion>,
    ) -> Result<AgentOutput> {
        match completion {
            Ok(completion) => {
                if let Some(output) = completion.output() {
                    let _ = self.events.send(AppServerEvent::TurnOutput {
                        output: Box::new(output.clone()),
                    });
                } else if let Some(error) = completion.error() {
                    let _ = self.events.send(AppServerEvent::Error {
                        message: format!("{error:#}"),
                    });
                }
                completion.into_result()
            }
            Err(error) => {
                let message = format!("{error:#}");
                let _ = self.events.send(AppServerEvent::Error {
                    message: message.clone(),
                });
                Err(error)
            }
        }
    }

    pub async fn clear_history(&self) -> Result<()> {
        self.runtime.clear_history().await
    }

    pub async fn set_permission_mode(&self, mode: PermissionMode) {
        self.runtime.set_permission_mode(mode).await;
        self.config.write().await.permissions.mode = mode;
    }

    pub async fn permission_mode(&self) -> PermissionMode {
        self.runtime.permission_mode().await
    }

    pub async fn set_model_name(&self, model: String) {
        self.runtime.set_model_name(model).await;
    }

    pub async fn set_reasoning_enabled(&self, enabled: bool) {
        self.runtime.set_reasoning_enabled(enabled).await;
    }

    pub async fn set_reasoning_effort(&self, effort: Option<String>) {
        self.runtime.set_reasoning_effort(effort).await;
    }

    /// Обновляет секцию `[web]` конфига (in-memory + запись в файл). Переданные
    /// `None`-поля не трогаем — патчим только то, что прислали.
    pub async fn set_web_config(&self, tool_cards_collapsed: Option<bool>) -> Result<()> {
        {
            let mut config = self.config.write().await;
            if let Some(value) = tool_cards_collapsed {
                config.web.tool_cards_collapsed = value;
            }
        }
        self.persist_web_config().await
    }

    /// Пишет [web] обратно в файл конфига, сохраняя комментарии и форматирование
    /// (toml_edit). Если config_path не задан или это директория — только память.
    async fn persist_web_config(&self) -> Result<()> {
        let Some(path) = self.config_path.clone() else {
            return Ok(());
        };
        if tokio::fs::metadata(&path)
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
        {
            return Ok(());
        }
        let web = self.config.read().await.web.clone();
        let mut doc = read_toml_document_or_empty(&path).await?;
        if !doc.contains_key("web") {
            doc["web"] = toml_edit::table();
        }
        doc["web"]["tool_cards_collapsed"] = toml_edit::value(web.tool_cards_collapsed);
        tokio::fs::write(&path, doc.to_string()).await?;
        Ok(())
    }

    pub async fn config_summary(&self) -> Value {
        let mode = self.permission_mode().await;
        let model_ref = self.runtime.model_ref().await;
        let reasoning = self.runtime.reasoning().await;
        let module_epoch = self.runtime.module_epoch().await;
        let config = self.config.read().await.clone();
        let effort_options = configured_reasoning_effort_options(&config, &model_ref, &reasoning);
        let tools = self.runtime.tool_entries().await;
        let config_files = config_files(self.config_path.as_deref());
        let model_options = configured_model_options(&config);
        json!({
            "display_text": render_config_summary(
                &config,
                self.config_path.as_deref(),
                &self.cwd,
                mode,
                &tools,
                module_epoch,
            ),
            "config_path": self
                .config_path
                .as_deref()
                .map(|path| path.display().to_string()),
            "config_files": config_files
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>(),
            "cwd": self.cwd.display().to_string(),
            "session_dir": self
                .runtime
                .session_dir()
                .map(|path| path.display().to_string()),
            "profile": config.profile.name,
            "model": {
                "provider": model_ref.provider.clone(),
                "name": model_ref.model.clone(),
                "label": format!("{}/{}", model_ref.provider, model_ref.model),
            },
            "model_options": model_options
                .iter()
                .map(|model| json!({
                    "provider": model.provider.clone(),
                    "name": model.model.clone(),
                    "label": format!("{}/{}", model.provider, model.model),
                }))
                .collect::<Vec<_>>(),
            "reasoning": {
                "enabled": reasoning.effort.is_some() || reasoning.summary || reasoning.budget_tokens.is_some(),
                "effort": reasoning.effort,
                "effort_options": effort_options,
                "summary": reasoning.summary,
                "budget_tokens": reasoning.budget_tokens,
            },
            "permission_mode": format!("{mode:?}"),
            "web": {
                "tool_cards_collapsed": config.web.tool_cards_collapsed,
            },
            "module_epoch": module_epoch.as_u64(),
            "modules": module_summary(&config),
            "tools_enabled": config.tools.enabled,
            "registered_tools": tools
                .iter()
                .map(|(source, spec)| json!({
                    "name": spec.name,
                    "source": source.label(),
                    "safety": format!("{:?}", spec.safety),
                    "description": spec.description,
                }))
                .collect::<Vec<_>>(),
            "components": config.components.iter().map(|(component_id, component)| json!({
                "id": component_id,
                "exports": component.exports().map(|(slot, module_id, _)| json!({
                    "slot": slot,
                    "module_id": module_id,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }

    pub async fn config_builder_snapshot(&self) -> ConfigBuilderSnapshot {
        let topology = self.topology_snapshot().await;
        let config = self.config.read().await.clone();
        config_builder_snapshot_from_topology(&topology, &config)
    }

    pub async fn set_config_builder(
        &self,
        modules: BTreeMap<String, String>,
        module_config: BTreeMap<String, BTreeMap<String, Value>>,
        tools_enabled: Option<Vec<String>>,
        active_provider: Option<String>,
        permission_mode: Option<PermissionMode>,
    ) -> Result<ConfigBuilderSnapshot> {
        let current_plan = self.runtime.assembly_plan().await;
        validate_config_builder_modules(&modules, current_plan.catalog_entries())?;

        let mut next_config = self.config.read().await.clone();
        if let Some(active_provider) = &active_provider {
            validate_config_builder_provider(active_provider, &next_config)?;
        }
        let previous_active_provider = next_config.active_provider.clone();
        for (slot, module_id) in modules {
            set_module_slot(&mut next_config.modules, &slot, module_id)?;
        }
        for (slot, values) in module_config {
            next_config.module_config.insert(slot, values);
        }
        if let Some(tools_enabled) = tools_enabled {
            next_config.tools.enabled = tools_enabled;
        }
        // Смену provider применяем к runtime model_ref только при фактическом
        // изменении: иначе save builder-а сбрасывал бы model, переключённую
        // на лету через POST /model.
        let provider_changed = active_provider
            .as_ref()
            .is_some_and(|provider| provider != &previous_active_provider);
        if let Some(active_provider) = active_provider {
            next_config.active_provider = active_provider;
        }
        if let Some(mode) = permission_mode {
            next_config.permissions.mode = mode;
        }
        validate_module_config_toml(&next_config.module_config)?;

        let assembly =
            prepare_assembly(&next_config, &self.cwd, self.config_path.as_deref()).await?;
        let target_path = config_builder_target_path(self.config_path.as_deref())
            .ok_or_else(|| anyhow!("config path is not available; cannot persist config"))?;
        persist_config_builder(&target_path, &next_config).await?;

        let config_snapshot = SessionConfigSnapshot::from_runtime_config(
            &next_config,
            assembly.registry(),
            next_config.permissions.mode,
        );
        let model_ref = provider_changed
            .then(|| {
                next_config
                    .active_model_config()
                    .map(|model| model.model_ref())
            })
            .transpose()?;
        let report = self
            .runtime
            .reload_assembly_with_effective_settings(
                assembly,
                Some(config_snapshot),
                model_ref,
                permission_mode,
            )
            .await?;
        *self.config.write().await = next_config;
        let _ = self.events.send(AppServerEvent::ModulesReloaded {
            old_epoch: report.old_epoch,
            new_epoch: report.new_epoch,
            tool_names: report.tool_names.clone(),
        });

        Ok(self.config_builder_snapshot().await)
    }

    pub async fn topology_snapshot(&self) -> TopologySnapshot {
        let mode = self.permission_mode().await;
        let (module_epoch, plan, tools) = self.runtime.assembly_observation().await;
        build_topology_snapshot(TopologyBuildInput {
            plan: &plan,
            tools: &tools,
            module_epoch,
            permission_mode: mode,
            extra_warnings: Vec::new(),
        })
    }

    pub async fn assembly_plan(&self) -> AssemblyPlan {
        self.runtime.assembly_plan().await
    }

    pub fn session_summaries(&self) -> Result<Vec<AppSessionSummary>> {
        let Some(config_path) = self.config_path.as_deref() else {
            return Ok(Vec::new());
        };
        list_session_summaries(&config_store_root(config_path))
    }

    pub fn workspace_session_summaries(&self) -> Result<Vec<AppSessionSummary>> {
        let Some(config_path) = self.config_path.as_deref() else {
            return Ok(Vec::new());
        };
        list_workspace_session_summaries(&config_store_root(config_path), &self.cwd)
    }

    pub async fn delete_workspace_session(&self, session_dir: PathBuf) -> Result<bool> {
        let Some(config_path) = self.config_path.as_deref() else {
            return Ok(false);
        };
        delete_workspace_session(&config_store_root(config_path), &self.cwd, session_dir).await
    }

    pub fn is_session_dir(&self, session_dir: &Path) -> bool {
        let Some(active_dir) = self.runtime.session_dir() else {
            return false;
        };
        normalize_session_dir_path(session_dir.to_path_buf())
            .is_ok_and(|session_dir| paths_equal(active_dir, &session_dir))
    }

    pub async fn transcript(&self) -> Result<Vec<AppTranscriptMessage>> {
        let mut transcript = if let Some(projection) = self.runtime.session_projection()? {
            let live_turn_id = self
                .runtime
                .active_turn_id()
                .filter(|turn_id| projection.unsettled_turns.contains(turn_id));
            journal_transcript_messages(&projection, live_turn_id)
        } else {
            transcript_messages(&self.runtime.history().await)
        };
        // Хвост незавершённого хода: history пополняется только при коммите
        // хода, настриманный текст и tool-вызовы до тех пор живут в прогрессе.
        transcript.extend(self.turn_progress.lock().await.snapshot());
        Ok(transcript)
    }

    pub async fn pending_requests(&self) -> AppPendingRequests {
        let mut approvals = self
            .pending_approvals
            .lock()
            .await
            .values()
            .map(|entry| entry.request.clone())
            .collect::<Vec<_>>();
        // Хронология очереди: seq присваивает forwarder; approval_id даёт
        // детерминированный порядок при равных seq в тестовых fixtures.
        approvals.sort_by(|left, right| {
            left.seq
                .cmp(&right.seq)
                .then_with(|| left.approval_id.cmp(&right.approval_id))
        });

        let mut user_inputs = self
            .pending_user_inputs
            .lock()
            .await
            .values()
            .map(|entry| entry.request.clone())
            .collect::<Vec<_>>();
        // Хронология очереди: seq присваивает forwarder; request_id даёт
        // детерминированный порядок при равных seq в тестовых fixtures.
        user_inputs.sort_by(|left, right| {
            left.seq
                .cmp(&right.seq)
                .then_with(|| left.request_id.cmp(&right.request_id))
        });

        let queued_user_messages = self
            .runtime
            .queued_user_messages()
            .await
            .into_iter()
            .map(|(message_id, text)| AppQueuedUserMessage::new(message_id, text))
            .collect();

        AppPendingRequests::new(approvals, user_inputs)
            .with_queued_user_messages(queued_user_messages)
    }

    pub async fn context_map_snapshot(
        &self,
        activity: Option<AppSessionActivity>,
    ) -> Result<AppContextMapSnapshot> {
        let session_dir = self.session_dir_path();
        let session_id = Some(self.runtime.session_id());
        let history = self.runtime.history().await;
        let event_log_path = self.context_event_log_path(&self.cwd).await;
        build_context_map_snapshot(ContextMapInput {
            session_dir,
            session_id,
            workspace_path: Some(self.cwd.clone()),
            activity,
            history,
            event_log_path,
            diagnostics: Vec::new(),
        })
    }

    pub async fn context_map_snapshot_for_session_dir(
        &self,
        session_dir: PathBuf,
        activity: Option<AppSessionActivity>,
    ) -> Result<AppContextMapSnapshot> {
        let session_dir = crate::core::canonicalize_session_dir_path(session_dir)?;
        let session_store = SessionStore::open(session_dir.clone())?;
        let history = session_store.load_messages()?;
        let session_id = session_store.session_id();
        let workspace_path = session_store.workspace_path()?;
        let event_log_path = self.context_event_log_path(&workspace_path).await;
        build_context_map_snapshot(ContextMapInput {
            session_dir: Some(session_dir),
            session_id: Some(session_id),
            workspace_path: Some(workspace_path),
            activity,
            history,
            event_log_path,
            diagnostics: Vec::new(),
        })
    }

    async fn context_event_log_path(&self, cwd: &Path) -> PathBuf {
        let config = self.config.read().await;
        crate::core::event_log_path(&config.event_log.path, self.config_path.as_deref(), cwd)
    }

    pub async fn has_pending_approval(&self, approval_id: &str) -> bool {
        self.pending_approvals
            .lock()
            .await
            .contains_key(approval_id)
    }

    pub async fn has_pending_user_input(&self, request_id: &str) -> bool {
        self.pending_user_inputs
            .lock()
            .await
            .contains_key(request_id)
    }

    pub async fn session_activity(&self, running_run_ids: Vec<String>) -> AppSessionActivity {
        let pending_approvals = self.pending_approvals.lock().await.len();
        let pending_user_inputs = self.pending_user_inputs.lock().await.len();
        AppSessionActivity::from_running_run_ids(
            running_run_ids,
            pending_approvals,
            pending_user_inputs,
        )
    }

    pub async fn respond_approval(
        &self,
        approval_id: &str,
        approved: bool,
        note: Option<String>,
        cache: ApprovalCacheScope,
    ) -> Result<()> {
        approvals::resolve_pending_approval(
            &self.pending_approvals,
            approval_id,
            ApprovalResponse::new(approved, note, cache),
        )
        .await
    }

    pub async fn respond_user_input(
        &self,
        request_id: &str,
        response: UserInputResponse,
    ) -> Result<()> {
        user_inputs::resolve_pending_user_input(&self.pending_user_inputs, request_id, response)
            .await
    }

    pub async fn shutdown(&self) {
        approvals::deny_pending_approvals(
            self.pending_approvals.clone(),
            "app-server shutting down".to_owned(),
        )
        .await;
        resolve_pending_user_inputs_empty(self.pending_user_inputs.clone()).await;
        let _ = self.events.send(AppServerEvent::Shutdown);
    }

    pub async fn reload_tools(&self) -> Result<RuntimeReloadReport> {
        let config = reload_tools_config(self.config_path.as_deref(), &self.config).await?;
        let assembly = prepare_assembly(&config, &self.cwd, self.config_path.as_deref()).await?;
        let config_snapshot = SessionConfigSnapshot::from_runtime_config(
            &config,
            assembly.registry(),
            config.permissions.mode,
        );
        let report = self
            .runtime
            .reload_assembly(assembly, Some(config_snapshot))
            .await?;
        *self.config.write().await = config;
        let _ = self.events.send(AppServerEvent::ModulesReloaded {
            old_epoch: report.old_epoch,
            new_epoch: report.new_epoch,
            tool_names: report.tool_names.clone(),
        });
        Ok(report)
    }
}

pub struct AgentAppServer;

impl AgentAppServer {
    pub async fn launch(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&Path>,
    ) -> Result<AppServerHandle> {
        Self::launch_inner(config, cwd, config_path, None, None).await
    }

    pub async fn launch_or_resume_latest(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&Path>,
    ) -> Result<AppServerHandle> {
        if let Some(session_dir) = latest_workspace_session_dir(config_path, &cwd)? {
            return Self::launch_resumed(config, cwd, config_path, session_dir).await;
        }
        Self::launch(config, cwd, config_path).await
    }

    pub async fn launch_resumed(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&Path>,
        session_dir: PathBuf,
    ) -> Result<AppServerHandle> {
        Self::launch_inner(config, cwd, config_path, None, Some(session_dir)).await
    }

    #[cfg(test)]
    pub(crate) async fn launch_with_module_catalog(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&Path>,
        module_catalog: ModuleCatalog,
    ) -> Result<AppServerHandle> {
        Self::launch_inner(config, cwd, config_path, Some(module_catalog), None).await
    }

    async fn launch_inner(
        config: AppConfig,
        mut cwd: PathBuf,
        config_path: Option<&Path>,
        module_catalog: Option<ModuleCatalog>,
        resume_session_dir: Option<PathBuf>,
    ) -> Result<AppServerHandle> {
        let resumed_session = resume_session_dir
            .map(normalize_session_dir_path)
            .transpose()?
            .map(SessionStore::open)
            .transpose()?;
        if let Some(session_store) = resumed_session.as_ref() {
            cwd = session_store.workspace_path()?;
        }

        let config_snapshot = Arc::new(RwLock::new(config.clone()));
        let config_path_snapshot = config_path.map(Path::to_path_buf);
        let cwd_snapshot = cwd.clone();
        let core_broadcast = Arc::new(BroadcastEventSink::new(1024));
        let event_log_path = crate::core::event_log_path(&config.event_log.path, config_path, &cwd);
        let jsonl_raw: Arc<dyn EventSink> = Arc::new(JsonlEventStore::new(event_log_path));
        // Дельты по умолчанию не пишем в durable log — они нужны UI (broadcast)
        // но засоряют файл на длинных ответах. `persist_deltas = true` в конфиге
        // включает полную запись.
        let jsonl: Arc<dyn EventSink> = if config.event_log.persist_deltas {
            jsonl_raw
        } else {
            Arc::new(FilteredEventSink::new(jsonl_raw, |event| {
                !is_streaming_delta(event)
            }))
        };
        let event_sink: Arc<dyn EventSink> =
            Arc::new(FanoutEventSink::new(vec![jsonl, core_broadcast.clone()]));

        let approval_timeout = Duration::from_millis(config.app_server.approval_timeout_ms);
        let (approval_transport, approval_rx) = ChannelApprovalTransport::new(32);
        let (user_input_transport, user_input_rx) = ChannelUserInputTransport::new(32);
        let mut builder = AgentRuntime::builder(config, cwd)
            .with_config_path(config_path)
            .with_event_sink(event_sink)
            .with_approval(Arc::new(approval_transport))
            .with_user_input(Arc::new(user_input_transport));
        if let Some(session_store) = resumed_session {
            builder = builder.resume_from_session_store(session_store, new_thread_id());
        }
        if let Some(module_catalog) = module_catalog {
            builder = builder.with_module_catalog(module_catalog);
        }
        let runtime = Arc::new(builder.build_async().await?);
        let (events, _) = broadcast::channel(1024);
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
        let pending_user_inputs = Arc::new(Mutex::new(HashMap::new()));
        let turn_progress = Arc::new(Mutex::new(TurnProgress::default()));

        spawn_runtime_event_forwarder(core_broadcast, events.clone(), turn_progress.clone());
        approvals::spawn_approval_forwarder(
            approval_rx,
            events.clone(),
            pending_approvals.clone(),
            approval_timeout,
        );
        user_inputs::spawn_user_input_forwarder(
            user_input_rx,
            events.clone(),
            pending_user_inputs.clone(),
            approval_timeout,
        );

        Ok(AppServerHandle {
            runtime,
            config: config_snapshot,
            config_path: config_path_snapshot,
            cwd: cwd_snapshot,
            events,
            pending_approvals,
            pending_user_inputs,
            turn_progress,
        })
    }
}

fn latest_workspace_session_dir(config_path: Option<&Path>, cwd: &Path) -> Result<Option<PathBuf>> {
    let Some(config_path) = config_path else {
        return Ok(None);
    };
    Ok(
        list_workspace_session_summaries(&config_store_root(config_path), cwd)?
            .into_iter()
            .next()
            .map(|session| session.session_dir),
    )
}

async fn reload_tools_config(
    config_path: Option<&Path>,
    current: &RwLock<AppConfig>,
) -> Result<AppConfig> {
    let mut config = current.read().await.clone();
    if let Some(path) = config_path {
        let loaded = AppConfig::load(Some(path)).await?;
        config.tools = loaded.tools;
    }
    Ok(config)
}

async fn prepare_assembly(
    config: &AppConfig,
    cwd: &Path,
    config_path: Option<&Path>,
) -> Result<PreparedAssembly> {
    let config = config.clone();
    let cwd = cwd.to_path_buf();
    let config_path = config_path.map(Path::to_path_buf);
    tokio::task::spawn_blocking(move || {
        PreparedAssembly::from_config(config, cwd, config_path.as_deref())
    })
    .await
    .map_err(|error| anyhow!("assembly builder blocking task failed: {error}"))?
}

fn spawn_runtime_event_forwarder(
    core_broadcast: Arc<BroadcastEventSink>,
    events: broadcast::Sender<AppServerEvent>,
    turn_progress: Arc<Mutex<TurnProgress>>,
) {
    let rx = core_broadcast.subscribe();
    spawn_runtime_event_forwarder_with_receiver(rx, events, turn_progress);
}

/// Отделено от `spawn_runtime_event_forwarder`, чтобы lag-путь можно было
/// детерминированно тестировать: receiver подписывается до переполнения.
fn spawn_runtime_event_forwarder_with_receiver(
    mut rx: broadcast::Receiver<EventEnvelope>,
    events: broadcast::Sender<AppServerEvent>,
    turn_progress: Arc<Mutex<TurnProgress>>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    turn_progress.lock().await.apply(&envelope);
                    let _ = events.send(AppServerEvent::Runtime {
                        envelope: Box::new(envelope),
                    });
                }
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    // Часть runtime-событий потеряна (переполнение ring):
                    // среди них могли быть ToolFinished/TurnFinished. Клиент
                    // обязан пересинхронизироваться, а не жить со «вечно
                    // бегущими» карточками — Error здесь не подходит, он
                    // означает «ход упал».
                    let _ = events.send(AppServerEvent::EventStreamLagged { count });
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests;
