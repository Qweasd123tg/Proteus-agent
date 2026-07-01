use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock, broadcast};
use uuid::Uuid;

use crate::{
    contracts::{
        ApprovalCacheScope, ApprovalResponse, CancellationToken, EventSink, FilteredEventSink,
        UserInputRequest, UserInputResponse, is_streaming_delta,
    },
    core::{
        AgentRuntime, AppConfig, BroadcastEventSink, BuiltinModuleCatalog,
        ChannelApprovalTransport, ChannelUserInputTransport, FanoutEventSink, JsonlEventStore,
        ModuleCatalogEntrySummary, PendingApproval, PendingUserInput, RuntimeReloadReport,
        SessionStore, TopologyBuildInput, TopologySnapshot, build_topology_snapshot,
        config_store_root, delete_workspace_session, list_session_summaries,
        list_workspace_session_summaries, normalize_session_dir_path, session_id_from_session_dir,
        session_workspace_from_session_dir,
    },
    domain::{AgentOutput, PermissionMode, new_thread_id},
};

mod approval_preview;
mod config_builder;
mod config_summary;
mod context_map;
pub mod http;
mod path_utils;
pub mod protocol;
pub mod stdio;
mod transcript;

use approval_preview::approval_preview_for;
pub use config_builder::{
    ConfigBuilderModule, ConfigBuilderModuleSelection, ConfigBuilderSlot, ConfigBuilderSnapshot,
    ConfigBuilderTool, ConfigBuilderWarning,
};
use config_builder::{
    config_builder_snapshot_from_topology, config_builder_target_path, persist_config_builder,
    read_toml_document_or_empty, set_module_slot, validate_config_builder_modules,
    validate_module_config_toml,
};
use config_summary::{
    config_files, configured_model_options, configured_reasoning_effort_options, module_summary,
    plugin_summary, render_config_summary,
};
use context_map::{ContextMapInput, build_context_map_snapshot};
use path_utils::paths_equal;
use transcript::transcript_messages;
pub use transcript::{AppTranscriptMessage, AppTranscriptTool};

// Wire protocol вынесен в proteus-contracts чтобы клиенты depend на него
// без зависимости на ядро. Здесь просто re-export для обратной
// совместимости внутри proteus-core.
pub use proteus_contracts::app_protocol::{
    AppApprovalId, AppApprovalPreview, AppApprovalRequest, AppContextBuildSnapshot,
    AppContextCompactionSnapshot, AppContextHistorySummary, AppContextMapSnapshot,
    AppContextToolSummary, AppContextUsageCategory, AppContextUsageSnapshot, AppPendingRequests,
    AppServerEvent, AppSessionActivity, AppUserInputRequestId, StdioOutput, StdioRequest,
};

struct PendingApprovalEntry {
    request: AppApprovalRequest,
    responder: tokio::sync::oneshot::Sender<ApprovalResponse>,
}

struct PendingUserInputEntry {
    request: UserInputRequest,
    responder: tokio::sync::oneshot::Sender<UserInputResponse>,
}

type PendingApprovalResponders = Arc<Mutex<HashMap<AppApprovalId, PendingApprovalEntry>>>;
type PendingUserInputResponders = Arc<Mutex<HashMap<AppUserInputRequestId, PendingUserInputEntry>>>;

#[derive(Clone)]
pub struct AppServerHandle {
    runtime: Arc<AgentRuntime>,
    config: Arc<RwLock<AppConfig>>,
    config_path: Option<PathBuf>,
    cwd: PathBuf,
    catalog_entries: Arc<RwLock<Vec<ModuleCatalogEntrySummary>>>,
    plugin_reports: Arc<RwLock<Vec<crate::core::PluginLoadReport>>>,
    events: broadcast::Sender<AppServerEvent>,
    pending_approvals: PendingApprovalResponders,
    pending_user_inputs: PendingUserInputResponders,
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
        match self.runtime.run_with_cancellation(text, cancellation).await {
            Ok(output) => {
                let _ = self.events.send(AppServerEvent::TurnOutput {
                    output: Box::new(output.clone()),
                });
                Ok(output)
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

    /// Обновляет секцию [web] конфига (in-memory + запись в файл). Переданные
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
        let plugin_reports = self.plugin_reports.read().await;
        json!({
            "display_text": render_config_summary(
                &config,
                self.config_path.as_deref(),
                &self.cwd,
                mode,
                &tools,
                &plugin_reports,
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
            "plugins": plugin_summary(&plugin_reports),
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
    ) -> Result<ConfigBuilderSnapshot> {
        let catalog_entries = self.catalog_entries.read().await.clone();
        validate_config_builder_modules(&modules, &catalog_entries)?;

        let mut next_config = self.config.read().await.clone();
        for (slot, module_id) in modules {
            set_module_slot(&mut next_config.modules, &slot, module_id)?;
        }
        for (slot, values) in module_config {
            next_config.module_config.insert(slot, values);
        }
        if let Some(tools_enabled) = tools_enabled {
            next_config.tools.enabled = tools_enabled;
        }
        validate_module_config_toml(&next_config.module_config)?;

        let (registry, plugin_reports, catalog_entries) =
            build_registry_and_plugin_reports(&next_config, &self.cwd)?;
        let target_path = config_builder_target_path(self.config_path.as_deref())
            .ok_or_else(|| anyhow!("config path is not available; cannot persist config"))?;
        persist_config_builder(&target_path, &next_config).await?;

        let report = self.runtime.reload_registry(registry).await?;
        *self.config.write().await = next_config;
        *self.plugin_reports.write().await = plugin_reports;
        *self.catalog_entries.write().await = catalog_entries;
        let _ = self.events.send(AppServerEvent::ModulesReloaded {
            old_epoch: report.old_epoch,
            new_epoch: report.new_epoch,
            tool_names: report.tool_names.clone(),
        });

        Ok(self.config_builder_snapshot().await)
    }

    pub async fn topology_snapshot(&self) -> TopologySnapshot {
        let mode = self.permission_mode().await;
        let module_epoch = self.runtime.module_epoch().await;
        let config = self.config.read().await.clone();
        let tools = self.runtime.tool_entries().await;
        let plugin_reports = self.plugin_reports.read().await;
        let catalog_entries = self.catalog_entries.read().await;
        build_topology_snapshot(TopologyBuildInput {
            config: &config,
            config_path: self.config_path.as_deref(),
            cwd: &self.cwd,
            catalog_entries: &catalog_entries,
            tools: &tools,
            plugin_reports: &plugin_reports,
            module_epoch,
            permission_mode: mode,
            extra_warnings: Vec::new(),
        })
    }

    pub fn session_summaries(&self) -> Result<Vec<crate::core::SessionSummary>> {
        let Some(config_path) = self.config_path.as_deref() else {
            return Ok(Vec::new());
        };
        list_session_summaries(&config_store_root(config_path))
    }

    pub fn workspace_session_summaries(&self) -> Result<Vec<crate::core::SessionSummary>> {
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

    pub async fn transcript(&self) -> Vec<AppTranscriptMessage> {
        transcript_messages(&self.runtime.history().await)
    }

    pub async fn pending_requests(&self) -> AppPendingRequests {
        let mut approvals = self
            .pending_approvals
            .lock()
            .await
            .values()
            .map(|entry| entry.request.clone())
            .collect::<Vec<_>>();
        approvals.sort_by(|left, right| left.approval_id.cmp(&right.approval_id));

        let mut user_inputs = self
            .pending_user_inputs
            .lock()
            .await
            .values()
            .map(|entry| entry.request.clone())
            .collect::<Vec<_>>();
        user_inputs.sort_by(|left, right| left.request_id.cmp(&right.request_id));

        AppPendingRequests::new(approvals, user_inputs)
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
        let history = SessionStore::from_session_dir(session_dir.clone()).load_messages()?;
        let mut diagnostics = Vec::new();
        let session_id = match session_id_from_session_dir(&session_dir) {
            Ok(session_id) => Some(session_id),
            Err(error) => {
                diagnostics.push(format!("session metadata unavailable: {error}"));
                None
            }
        };
        let workspace_path = match session_workspace_from_session_dir(&session_dir) {
            Ok(path) => path,
            Err(error) => {
                diagnostics.push(format!("workspace metadata unavailable: {error}"));
                None
            }
        };
        let event_log_cwd = workspace_path.as_deref().unwrap_or(&self.cwd);
        let event_log_path = self.context_event_log_path(event_log_cwd).await;
        build_context_map_snapshot(ContextMapInput {
            session_dir: Some(session_dir),
            session_id,
            workspace_path,
            activity,
            history,
            event_log_path,
            diagnostics,
        })
    }

    async fn context_event_log_path(&self, cwd: &Path) -> PathBuf {
        let config = self.config.read().await;
        crate::core::runtime::event_log_path(
            &config.event_log.path,
            self.config_path.as_deref(),
            cwd,
        )
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

    pub async fn session_activity(&self, running_turn_ids: Vec<String>) -> AppSessionActivity {
        let pending_approvals = self.pending_approvals.lock().await.len();
        let pending_user_inputs = self.pending_user_inputs.lock().await.len();
        AppSessionActivity::from_running_turn_ids(
            running_turn_ids,
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
        let responder = self
            .pending_approvals
            .lock()
            .await
            .remove(approval_id)
            .ok_or_else(|| anyhow!("unknown approval id: {approval_id}"))?;
        responder
            .responder
            .send(ApprovalResponse::new(approved, note, cache))
            .map_err(|_| anyhow!("approval response channel dropped"))?;
        let _ = self.events.send(AppServerEvent::ApprovalResolved {
            approval_id: approval_id.to_owned(),
            approved,
        });
        Ok(())
    }

    pub async fn respond_user_input(
        &self,
        request_id: &str,
        response: UserInputResponse,
    ) -> Result<()> {
        let responder = self
            .pending_user_inputs
            .lock()
            .await
            .remove(request_id)
            .ok_or_else(|| anyhow!("unknown user input request id: {request_id}"))?;
        responder
            .responder
            .send(response)
            .map_err(|_| anyhow!("user input response channel dropped"))?;
        let _ = self.events.send(AppServerEvent::UserInputResolved {
            request_id: request_id.to_owned(),
        });
        Ok(())
    }

    pub async fn shutdown(&self) {
        deny_pending_approvals(
            self.pending_approvals.clone(),
            self.events.clone(),
            "app-server shutting down".to_owned(),
        )
        .await;
        deny_pending_user_inputs(
            self.pending_user_inputs.clone(),
            self.events.clone(),
            "app-server shutting down".to_owned(),
        )
        .await;
        let _ = self.events.send(AppServerEvent::Shutdown);
    }

    pub async fn cancel_pending_approvals(&self, note: String) {
        deny_pending_approvals(self.pending_approvals.clone(), self.events.clone(), note).await;
    }

    pub async fn cancel_pending_user_inputs(&self, note: String) {
        deny_pending_user_inputs(self.pending_user_inputs.clone(), self.events.clone(), note).await;
    }

    pub async fn reload_tools(&self) -> Result<RuntimeReloadReport> {
        let config = reload_tools_config(self.config_path.as_deref(), &self.config).await?;
        let (registry, plugin_reports, catalog_entries) =
            build_registry_and_plugin_reports(&config, &self.cwd)?;
        let report = self.runtime.reload_registry(registry).await?;
        *self.config.write().await = config;
        *self.plugin_reports.write().await = plugin_reports;
        *self.catalog_entries.write().await = catalog_entries;
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
    pub fn launch(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&Path>,
    ) -> Result<AppServerHandle> {
        Self::launch_inner(config, cwd, config_path, None, None)
    }

    pub fn launch_or_resume_latest(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&Path>,
    ) -> Result<AppServerHandle> {
        if let Some(session_dir) = latest_workspace_session_dir(config_path, &cwd)? {
            return Self::launch_resumed(config, cwd, config_path, session_dir);
        }
        Self::launch(config, cwd, config_path)
    }

    pub fn launch_resumed(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&Path>,
        session_dir: PathBuf,
    ) -> Result<AppServerHandle> {
        Self::launch_inner(config, cwd, config_path, None, Some(session_dir))
    }

    #[cfg(test)]
    pub(crate) fn launch_with_module_catalog(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&Path>,
        module_catalog: BuiltinModuleCatalog,
    ) -> Result<AppServerHandle> {
        Self::launch_inner(config, cwd, config_path, Some(module_catalog), None)
    }

    fn launch_inner(
        config: AppConfig,
        mut cwd: PathBuf,
        config_path: Option<&Path>,
        module_catalog: Option<BuiltinModuleCatalog>,
        resume_session_dir: Option<PathBuf>,
    ) -> Result<AppServerHandle> {
        let resume_session_dir = resume_session_dir
            .map(normalize_session_dir_path)
            .transpose()?;
        if let Some(session_dir) = resume_session_dir.as_deref()
            && let Some(workspace_path) =
                crate::core::session_workspace_from_session_dir(session_dir)?
        {
            cwd = workspace_path;
        }

        let config_snapshot = Arc::new(RwLock::new(config.clone()));
        let config_path_snapshot = config_path.map(Path::to_path_buf);
        let cwd_snapshot = cwd.clone();
        let (module_catalog, plugin_reports, catalog_entries) = match module_catalog {
            Some(catalog) => {
                let catalog_entries = catalog.entry_summaries();
                (Some(catalog), Vec::new(), catalog_entries)
            }
            None => {
                let (catalog, reports) = load_module_catalog_with_reports();
                let catalog_entries = catalog.entry_summaries();
                (Some(catalog), reports, catalog_entries)
            }
        };
        let core_broadcast = Arc::new(BroadcastEventSink::new(1024));
        let event_log_path =
            crate::core::runtime::event_log_path(&config.event_log.path, config_path, &cwd);
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
        if let Some(session_dir) = resume_session_dir {
            let session_id = session_id_from_session_dir(&session_dir)?;
            builder = builder.resume_from_session_dir(session_dir, session_id, new_thread_id());
        }
        if let Some(module_catalog) = module_catalog {
            builder = builder.with_module_catalog(module_catalog);
        }
        let runtime = Arc::new(builder.build()?);
        let (events, _) = broadcast::channel(1024);
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
        let pending_user_inputs = Arc::new(Mutex::new(HashMap::new()));

        spawn_runtime_event_forwarder(core_broadcast, events.clone());
        spawn_approval_forwarder(
            approval_rx,
            events.clone(),
            pending_approvals.clone(),
            approval_timeout,
        );
        spawn_user_input_forwarder(
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
            catalog_entries: Arc::new(RwLock::new(catalog_entries)),
            plugin_reports: Arc::new(RwLock::new(plugin_reports)),
            events,
            pending_approvals,
            pending_user_inputs,
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
            .find(|session| session.resumable)
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

fn load_module_catalog_with_reports() -> (BuiltinModuleCatalog, Vec<crate::core::PluginLoadReport>)
{
    let mut catalog = BuiltinModuleCatalog::new();
    let reports = crate::core::default_plugins_dir()
        .map(|plugins_dir| crate::core::load_plugins_from_dir(&plugins_dir, &mut catalog))
        .unwrap_or_default();
    (catalog, reports)
}

fn build_registry_and_plugin_reports(
    config: &AppConfig,
    cwd: &Path,
) -> Result<(
    crate::core::BuiltinRegistry,
    Vec<crate::core::PluginLoadReport>,
    Vec<ModuleCatalogEntrySummary>,
)> {
    let (catalog, reports) = load_module_catalog_with_reports();
    let catalog_entries = catalog.entry_summaries();
    let registry = crate::core::BuiltinRegistry::from_catalog(config, cwd.to_path_buf(), catalog)?;
    Ok((registry, reports, catalog_entries))
}

fn spawn_runtime_event_forwarder(
    core_broadcast: Arc<BroadcastEventSink>,
    events: broadcast::Sender<AppServerEvent>,
) {
    tokio::spawn(async move {
        let mut rx = core_broadcast.subscribe();
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    let _ = events.send(AppServerEvent::Runtime {
                        envelope: Box::new(envelope),
                    });
                }
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    let _ = events.send(AppServerEvent::Error {
                        message: format!("runtime event stream lagged by {count} events"),
                    });
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn spawn_approval_forwarder(
    mut approval_rx: tokio::sync::mpsc::Receiver<PendingApproval>,
    events: broadcast::Sender<AppServerEvent>,
    pending_approvals: PendingApprovalResponders,
    approval_timeout: Duration,
) {
    tokio::spawn(async move {
        while let Some(PendingApproval { request, responder }) = approval_rx.recv().await {
            let approval_id = Uuid::new_v4().to_string();
            let preview = approval_preview_for(&request.call, &request.cwd);
            let app_request = AppApprovalRequest::new(
                approval_id.clone(),
                request.call,
                request.cwd,
                request.reason,
                request.tool_spec,
            )
            .with_preview(preview);
            pending_approvals.lock().await.insert(
                approval_id.clone(),
                PendingApprovalEntry {
                    request: app_request.clone(),
                    responder,
                },
            );
            let _ = events.send(AppServerEvent::ApprovalRequested {
                request: Box::new(app_request),
            });

            if !approval_timeout.is_zero() {
                spawn_approval_timeout(
                    approval_id,
                    pending_approvals.clone(),
                    events.clone(),
                    approval_timeout,
                );
            }
        }
    });
}

fn spawn_approval_timeout(
    approval_id: AppApprovalId,
    pending_approvals: PendingApprovalResponders,
    events: broadcast::Sender<AppServerEvent>,
    approval_timeout: Duration,
) {
    tokio::spawn(async move {
        tokio::time::sleep(approval_timeout).await;
        let entry = pending_approvals.lock().await.remove(&approval_id);
        if let Some(entry) = entry {
            let timeout_ms = approval_timeout.as_millis() as u64;
            let _ = entry.responder.send(ApprovalResponse::deny(format!(
                "approval request timed out after {timeout_ms}ms"
            )));
            let _ = events.send(AppServerEvent::ApprovalResolved {
                approval_id,
                approved: false,
            });
        }
    });
}

fn spawn_user_input_forwarder(
    mut user_input_rx: tokio::sync::mpsc::Receiver<PendingUserInput>,
    events: broadcast::Sender<AppServerEvent>,
    pending_user_inputs: PendingUserInputResponders,
    timeout: Duration,
) {
    tokio::spawn(async move {
        while let Some(PendingUserInput { request, responder }) = user_input_rx.recv().await {
            let request_id = request.request_id.clone();
            pending_user_inputs.lock().await.insert(
                request_id.clone(),
                PendingUserInputEntry {
                    request: request.clone(),
                    responder,
                },
            );
            let _ = events.send(AppServerEvent::UserInputRequested {
                request: Box::new(request),
            });

            if !timeout.is_zero() {
                spawn_user_input_timeout(
                    request_id,
                    pending_user_inputs.clone(),
                    events.clone(),
                    timeout,
                );
            }
        }
    });
}

fn spawn_user_input_timeout(
    request_id: AppUserInputRequestId,
    pending_user_inputs: PendingUserInputResponders,
    events: broadcast::Sender<AppServerEvent>,
    timeout: Duration,
) {
    tokio::spawn(async move {
        tokio::time::sleep(timeout).await;
        let entry = pending_user_inputs.lock().await.remove(&request_id);
        if let Some(entry) = entry {
            let _ = entry.responder.send(UserInputResponse::empty());
            let _ = events.send(AppServerEvent::UserInputResolved { request_id });
        }
    });
}

async fn deny_pending_approvals(
    pending_approvals: PendingApprovalResponders,
    events: broadcast::Sender<AppServerEvent>,
    note: String,
) {
    let pending = std::mem::take(&mut *pending_approvals.lock().await);
    for (approval_id, entry) in pending {
        let _ = entry.responder.send(ApprovalResponse::deny(note.clone()));
        let _ = events.send(AppServerEvent::ApprovalResolved {
            approval_id,
            approved: false,
        });
    }
}

async fn deny_pending_user_inputs(
    pending_user_inputs: PendingUserInputResponders,
    events: broadcast::Sender<AppServerEvent>,
    _note: String,
) {
    let pending = std::mem::take(&mut *pending_user_inputs.lock().await);
    for (request_id, entry) in pending {
        let _ = entry.responder.send(UserInputResponse::empty());
        let _ = events.send(AppServerEvent::UserInputResolved { request_id });
    }
}

#[cfg(test)]
mod tests;
