use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, anyhow};
use serde_json::Value;
use tokio::sync::{Mutex, RwLock, broadcast};

use crate::{
    contracts::{CancellationToken, EventSink, FilteredEventSink, is_streaming_delta},
    core::{
        AgentRuntime, AppConfig, AssemblyPlan, ChannelApprovalTransport, ChannelUserInputTransport,
        FanoutEventSink, JsonlEventStore, ModuleCatalog, PreparedAssembly, ReservedRunCompletion,
        SessionStore, TopologyBuildInput, TopologySnapshot, UserMessageReservation,
        build_topology_snapshot, config_store_root, delete_workspace_session,
        list_session_summaries, list_workspace_session_summaries, normalize_session_dir_path,
    },
    domain::{AgentOutput, PermissionMode, SessionId, new_thread_id},
};

pub mod acp;
mod addons;
mod approval_preview;
mod approvals;
mod commands;
mod config_builder;
mod config_history;
mod handle;
mod profile;
mod profile_watch;
pub use handle::{AppServerHandle, AppServerState};
mod config_summary;
mod context_map;
mod control_plane;
mod events;
use events::{AppEventPublisher, RuntimeEventSink};
pub mod http;
mod model_metadata;
mod model_selection;
mod path_utils;
mod queued_messages;
mod runs;
pub mod stdio;
mod transcript;
mod turn_progress;
mod usage;
mod user_inputs;

pub use config_builder::{
    ConfigBuilderModule, ConfigBuilderModuleSelection, ConfigBuilderProvider, ConfigBuilderSlot,
    ConfigBuilderSnapshot, ConfigBuilderState, ConfigBuilderTool, ConfigBuilderWarning,
};
pub use config_history::{ConfigHistory, ConfigRevision};
use context_map::{ContextMapInput, build_context_map_snapshot};
use path_utils::paths_equal;
pub(crate) use transcript::journal_transcript_messages;
use transcript::transcript_messages;
pub use transcript::{AppTranscriptMessage, AppTranscriptSubagent, AppTranscriptTool};
use turn_progress::TurnProgress;
mod startup;

// Публичный app-server façade экспортирует canonical wire types из contracts;
// их определения не дублируются в core.
pub use proteus_contracts::app_protocol::{
    AppApprovalId, AppApprovalPreview, AppApprovalRequest, AppContextBuildSnapshot,
    AppContextCompactionSnapshot, AppContextHistorySummary, AppContextMapSnapshot,
    AppContextToolSummary, AppContextUsageCategory, AppContextUsageSnapshot, AppHistorySummary,
    AppPendingRequests, AppQueuedUserMessage, AppRememberResult, AppServerEvent,
    AppSessionActivity, AppSessionSummary, AppUserInputRequestId, StdioOutput, StdioRequest,
};

impl AppServerHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<AppServerEvent> {
        self.events.subscribe()
    }

    pub(crate) fn subscribe_session(&self) -> events::AppSubscription {
        self.events.subscribe_session()
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
        match self
            .admit_user_message(None, text, Default::default(), cancellation, false)
            .await?
        {
            runs::SendDispatch::Started(rx) => {
                rx.await.map_err(|_| anyhow!("run ended without result"))?
            }
            runs::SendDispatch::Queued(_) => Err(anyhow!("message queued behind active run")),
        }
    }

    pub(crate) async fn reserve_user_message(
        &self,
        input: impl Into<crate::domain::UserMessageInput>,
        options: crate::domain::RunOptions,
    ) -> Result<UserMessageReservation> {
        let reservation = self
            .runtime
            .reserve_user_message_with_options(input, options)
            .await?;
        if let UserMessageReservation::Start(reserved) = &reservation {
            let _ = self.events.send(AppServerEvent::UserMessageSubmitted {
                text: reserved.text.clone(),
                images: reserved
                    .message
                    .parts
                    .iter()
                    .filter_map(|p| match &p.payload {
                        crate::model_standard::ContentPart::Image { image } => Some(image.clone()),
                        _ => None,
                    })
                    .collect(),
            });
        }
        Ok(reservation)
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

    pub(crate) async fn history_summary(&self) -> AppHistorySummary {
        AppHistorySummary::new(self.runtime.history_len().await)
    }

    pub(crate) async fn remember(
        &self,
        kind: String,
        content: String,
    ) -> Result<AppRememberResult> {
        let item = crate::domain::MemoryItem::new(&kind, &content, Value::Null);
        self.runtime
            .remember(item, CancellationToken::new())
            .await?;
        Ok(AppRememberResult::new(kind, content))
    }

    pub async fn set_permission_mode(&self, mode: PermissionMode) {
        self.runtime.set_permission_mode(mode).await;
        self.config.write().await.permissions.mode = mode;
    }

    pub async fn permission_mode(&self) -> PermissionMode {
        self.runtime.permission_mode().await
    }

    pub async fn set_model_name(&self, model: String) -> Result<()> {
        self.runtime.set_model_name(model).await
    }

    pub async fn set_reasoning_enabled(&self, enabled: bool) {
        self.runtime.set_reasoning_enabled(enabled).await;
    }

    pub async fn set_reasoning_effort(&self, effort: Option<String>) -> Result<()> {
        self.runtime.set_reasoning_effort(effort).await
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
        Ok(self.events.session_snapshot()?.transcript)
    }

    pub async fn context_map_snapshot(
        &self,
        activity: Option<AppSessionActivity>,
    ) -> Result<AppContextMapSnapshot> {
        let session_dir = self.session_dir_path();
        let session_id = Some(self.runtime.session_id());
        let history = self.runtime.history().await;
        let event_log_path = self.context_event_log_path(&self.cwd).await;
        let input = ContextMapInput {
            session_dir,
            session_id,
            workspace_path: Some(self.cwd.clone()),
            activity,
            history,
            event_log_path,
            diagnostics: Vec::new(),
        };
        tokio::task::spawn_blocking(move || build_context_map_snapshot(input))
            .await
            .map_err(|error| anyhow!("context map task failed: {error}"))?
    }

    pub async fn context_map_snapshot_for_session_dir(
        &self,
        session_dir: PathBuf,
        activity: Option<AppSessionActivity>,
    ) -> Result<AppContextMapSnapshot> {
        let session_dir = crate::core::canonicalize_session_dir_path(session_dir)?;
        let config = self.config.read().await;
        let event_log_config_path = config.event_log.path.clone();
        drop(config);
        let config_path = self.config_path.clone();
        tokio::task::spawn_blocking(move || {
            let session_store = SessionStore::open(session_dir.clone())?;
            let workspace_path = session_store.workspace_path()?;
            let event_log_path = crate::core::event_log_path(
                &event_log_config_path,
                config_path.as_deref(),
                &workspace_path,
            );
            build_context_map_snapshot(ContextMapInput {
                session_dir: Some(session_dir),
                session_id: Some(session_store.session_id()),
                workspace_path: Some(workspace_path),
                activity,
                history: session_store.load_messages()?,
                event_log_path,
                diagnostics: Vec::new(),
            })
        })
        .await
        .map_err(|error| anyhow!("context map task failed: {error}"))?
    }

    async fn context_event_log_path(&self, cwd: &Path) -> PathBuf {
        let config = self.config.read().await;
        crate::core::event_log_path(&config.event_log.path, self.config_path.as_deref(), cwd)
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
        let runtime_events = Arc::new(RuntimeEventSink::default());
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
            Arc::new(FanoutEventSink::new(vec![jsonl, runtime_events.clone()]));

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
        let events = AppEventPublisher::new(
            1024,
            runtime.session_id(),
            Some(runtime.subscribe_queued_user_messages()),
        );
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
        let pending_user_inputs = Arc::new(Mutex::new(HashMap::new()));
        events.attach_runtime(&runtime, completed_transcript(runtime.clone()).await?);
        assert!(runtime_events.0.set(events.clone()).is_ok());
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

        let handle = AppServerHandle {
            inner: Arc::new(AppServerState {
                runtime,
                config: config_snapshot,
                config_path: config_path_snapshot,
                cwd: cwd_snapshot,
                events,
                pending_approvals,
                pending_user_inputs,
                runs: Arc::new(Mutex::new(runs::RunRegistry::default())),
                profile_stop: CancellationToken::new(),
                profile_error: Mutex::new(None),
            }),
        };
        handle.command_catalog().await?;
        profile_watch::start(&handle).await;
        Ok(handle)
    }
}

/// Build the completed transcript once per committed turn. The journal is the
/// authority for persisted sessions; disk replay and projection run outside
/// the event and admission locks and off the async executor.
async fn completed_transcript(runtime: Arc<AgentRuntime>) -> Result<Vec<AppTranscriptMessage>> {
    if runtime.session_dir().is_none() {
        return Ok(transcript_messages(&runtime.history().await));
    }
    tokio::task::spawn_blocking(move || {
        let projection = runtime
            .session_projection()?
            .ok_or_else(|| anyhow!("persisted session has no journal projection"))?;
        Ok(journal_transcript_messages(&projection, None))
    })
    .await
    .map_err(|error| anyhow!("transcript projection task failed: {error}"))?
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

#[cfg(test)]
mod tests;
