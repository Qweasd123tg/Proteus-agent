use std::{
    collections::{BTreeSet, HashMap},
    path::{Component, Path, PathBuf},
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
        ToolSource, UserInputRequest, UserInputResponse, is_streaming_delta,
    },
    core::{
        AgentRuntime, AppConfig, BroadcastEventSink, BuiltinModuleCatalog,
        ChannelApprovalTransport, ChannelUserInputTransport, FanoutEventSink, JsonlEventStore,
        ModuleCatalogEntrySummary, PendingApproval, PendingUserInput, RuntimeReloadReport,
        TopologyBuildInput, TopologySnapshot, build_topology_snapshot, config_store_root,
        delete_workspace_session, list_session_summaries, list_workspace_session_summaries,
        normalize_session_dir_path, session_id_from_session_dir,
    },
    domain::{AgentOutput, PermissionMode, ToolCall, new_thread_id},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

pub mod http;
pub mod protocol;
pub mod stdio;

// Wire protocol вынесен в proteus-contracts чтобы клиенты depend на него
// без зависимости на ядро. Здесь просто re-export для обратной
// совместимости внутри proteus-core.
pub use proteus_contracts::app_protocol::{
    AppApprovalId, AppApprovalPreview, AppApprovalRequest, AppPendingRequests, AppServerEvent,
    AppSessionActivity, AppUserInputRequestId, StdioOutput, StdioRequest,
};

const APPROVAL_PREVIEW_BODY_LIMIT: usize = 20_000;

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppTranscriptMessage {
    pub role: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<AppTranscriptTool>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppTranscriptTool {
    pub call_id: String,
    pub name: String,
    pub args: Value,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

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

    pub async fn session_activity(&self, running_turns: usize) -> AppSessionActivity {
        let pending_approvals = self.pending_approvals.lock().await.len();
        let pending_user_inputs = self.pending_user_inputs.lock().await.len();
        AppSessionActivity::from_counts(running_turns, pending_approvals, pending_user_inputs)
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

pub(super) fn transcript_messages(messages: &[CanonicalMessage]) -> Vec<AppTranscriptMessage> {
    let mut transcript = Vec::new();
    for message in messages {
        append_transcript_message(&mut transcript, message);
    }
    transcript
}

fn append_transcript_message(
    transcript: &mut Vec<AppTranscriptMessage>,
    message: &CanonicalMessage,
) {
    let role = transcript_role(&message.role).to_owned();
    let mut text_parts = Vec::new();
    for part in &message.parts {
        match part {
            ContentPart::Text { text }
            | ContentPart::ReasoningSummary { text }
            | ContentPart::Reasoning { text, signature: _ } => {
                if !text.trim().is_empty() {
                    text_parts.push(text.clone());
                }
            }
            ContentPart::ToolCall { call } => {
                flush_transcript_text(transcript, &role, &mut text_parts);
                transcript.push(AppTranscriptMessage {
                    role: "system".to_owned(),
                    text: String::new(),
                    tool: Some(AppTranscriptTool {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        args: call.args.clone(),
                        status: "running".to_owned(),
                        result: None,
                    }),
                });
            }
            ContentPart::ToolResult { result } => {
                flush_transcript_text(transcript, &role, &mut text_parts);
                append_transcript_tool_result(transcript, result);
            }
            _ => {}
        }
    }
    flush_transcript_text(transcript, &role, &mut text_parts);
}

fn flush_transcript_text(
    transcript: &mut Vec<AppTranscriptMessage>,
    role: &str,
    text_parts: &mut Vec<String>,
) {
    if text_parts.is_empty() {
        return;
    }
    transcript.push(AppTranscriptMessage {
        role: role.to_owned(),
        text: text_parts.join("\n\n"),
        tool: None,
    });
    text_parts.clear();
}

fn append_transcript_tool_result(
    transcript: &mut Vec<AppTranscriptMessage>,
    result: &crate::domain::ToolResult,
) {
    let status = if result.ok { "done" } else { "failed" }.to_owned();
    let result_text = result.text_or_status();
    if let Some(tool) = transcript
        .iter_mut()
        .rev()
        .filter_map(|message| message.tool.as_mut())
        .find(|tool| tool.call_id == result.call_id)
    {
        tool.status = status;
        tool.result = Some(result_text);
        return;
    }

    transcript.push(AppTranscriptMessage {
        role: "system".to_owned(),
        text: String::new(),
        tool: Some(AppTranscriptTool {
            call_id: result.call_id.clone(),
            name: "tool".to_owned(),
            args: Value::Null,
            status,
            result: Some(result_text),
        }),
    });
}

fn transcript_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System | MessageRole::Developer => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "system",
        _ => "system",
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

fn render_config_summary(
    config: &AppConfig,
    config_path: Option<&Path>,
    cwd: &Path,
    mode: PermissionMode,
    tools: &[(ToolSource, crate::domain::ToolSpec)],
    plugin_reports: &[crate::core::PluginLoadReport],
    module_epoch: crate::core::ModuleEpoch,
) -> String {
    let mut lines = Vec::new();
    lines.push("Config summary".to_owned());
    lines.push(format!(
        "config path: {}",
        config_path
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "(default discovery / none)".to_owned())
    ));
    let config_files = config_files(config_path);
    if !config_files.is_empty() {
        lines.push("config files:".to_owned());
        for path in config_files {
            lines.push(format!("  - {}", path.display()));
        }
    }
    lines.push(format!("cwd: {}", cwd.display()));
    lines.push(format!("profile: {}", config.profile.name));
    lines.push(format!("module epoch: {}", module_epoch.as_u64()));
    if let Ok(model) = config.active_model_config() {
        lines.push(format!("model: {}/{}", model.provider, model.model));
    }
    lines.push(format!("permission mode: {mode:?}"));
    lines.push("modules:".to_owned());
    lines.push(format!("  workflow: {}", config.modules.workflow));
    lines.push(format!("  context: {}", config.modules.context));
    lines.push(format!("  tool_exposure: {}", config.modules.tool_exposure));
    lines.push(format!("  policy: {}", config.modules.policy));
    lines.push(format!("  search: {}", config.modules.search));
    lines.push(format!("  patch: {}", config.modules.patch));
    lines.push(format!("  memory: {}", config.modules.memory));
    lines.push(format!("  memory_policy: {}", config.modules.memory_policy));
    lines.push(format!("  compactor: {}", config.modules.compactor));
    lines.push(format!("  renderer: {}", config.modules.renderer));

    lines.push("tools.enabled:".to_owned());
    if config.tools.enabled.is_empty() {
        lines.push("  (none)".to_owned());
    } else {
        for tool in &config.tools.enabled {
            lines.push(format!("  - {tool}"));
        }
    }

    lines.push("registered tools:".to_owned());
    if tools.is_empty() {
        lines.push("  (none)".to_owned());
    } else {
        for (source, spec) in tools {
            lines.push(format!(
                "  - {} [{} {:?}] {}",
                spec.name,
                source.label(),
                spec.safety,
                spec.description
            ));
        }
    }

    lines.push("plugins:".to_owned());
    if plugin_reports.is_empty() {
        lines.push("  (none found)".to_owned());
    } else {
        for report in plugin_reports {
            let (name, version, description) = plugin_display_fields(report);
            let status = match &report.result {
                Ok(_) => "loaded".to_owned(),
                Err(error) => format!("error: {}", first_line(&error.to_string())),
            };
            if description.is_empty() {
                lines.push(format!("  - {name} {version}: {status}"));
            } else {
                lines.push(format!("  - {name} {version}: {status} - {description}"));
            }
        }
    }

    lines.join("\n")
}

fn module_summary(config: &AppConfig) -> Vec<Value> {
    [
        ("workflow", config.modules.workflow.as_str()),
        ("context", config.modules.context.as_str()),
        ("tool_exposure", config.modules.tool_exposure.as_str()),
        ("policy", config.modules.policy.as_str()),
        ("search", config.modules.search.as_str()),
        ("patch", config.modules.patch.as_str()),
        ("memory", config.modules.memory.as_str()),
        ("memory_policy", config.modules.memory_policy.as_str()),
        ("compactor", config.modules.compactor.as_str()),
        ("renderer", config.modules.renderer.as_str()),
    ]
    .into_iter()
    .map(|(slot, id)| json!({ "slot": slot, "id": id }))
    .collect()
}

fn configured_model_options(config: &AppConfig) -> Vec<crate::domain::ModelRef> {
    let mut options = Vec::new();
    if let Ok(model) = config.active_model_config() {
        options.push(model.model_ref());
    }
    for profile in config.providers.values() {
        if let Ok(model) = profile.to_model_config() {
            let model_ref = model.model_ref();
            if !options.iter().any(|item| item == &model_ref) {
                options.push(model_ref);
            }
        }
    }
    options
}

fn configured_reasoning_effort_options(
    config: &AppConfig,
    active_model: &crate::domain::ModelRef,
    reasoning: &crate::domain::ReasoningConfig,
) -> Vec<String> {
    let mut options = Vec::new();
    for profile in matching_provider_profiles(config, active_model) {
        push_unique_strings(&mut options, &profile.reasoning_efforts);
    }

    if looks_like_deepseek(config, active_model) {
        push_unique(&mut options, "high");
        push_unique(&mut options, "max");
    }

    if let Some(effort) = reasoning.effort.as_deref() {
        push_unique(&mut options, effort);
    }

    options
}

fn matching_provider_profiles<'a>(
    config: &'a AppConfig,
    active_model: &crate::domain::ModelRef,
) -> Vec<&'a crate::core::ProviderProfileConfig> {
    let mut profiles = Vec::new();
    if let Some(profile) = active_provider_profile(config) {
        profiles.push(profile);
    }
    profiles.extend(config.providers.values().filter(|profile| {
        profile.provider == active_model.provider && profile.model == active_model.model
    }));
    profiles
}

fn active_provider_profile(config: &AppConfig) -> Option<&crate::core::ProviderProfileConfig> {
    if let Some(active_provider) = config
        .active_provider
        .as_ref()
        .filter(|provider| !provider.trim().is_empty())
    {
        return config.providers.get(active_provider);
    }
    config.providers.get("default")
}

fn looks_like_deepseek(config: &AppConfig, active_model: &crate::domain::ModelRef) -> bool {
    let model = active_model.model.to_ascii_lowercase();
    let provider = active_model.provider.to_ascii_lowercase();
    let provider_config = config
        .active_model_config()
        .ok()
        .map(|model| model.provider_config.to_string().to_ascii_lowercase())
        .unwrap_or_default();
    model.contains("deepseek")
        || provider.contains("deepseek")
        || provider_config.contains("deepseek")
}

fn push_unique_strings(options: &mut Vec<String>, values: &[String]) {
    for value in values {
        push_unique(options, value);
    }
}

fn push_unique(options: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() || options.iter().any(|item| item == value) {
        return;
    }
    options.push(value.to_owned());
}

fn config_files(config_path: Option<&Path>) -> Vec<PathBuf> {
    let Some(path) = config_path else {
        return Vec::new();
    };
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| matches!(extension, "toml" | "json"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn plugin_summary(reports: &[crate::core::PluginLoadReport]) -> Vec<Value> {
    reports
        .iter()
        .map(|report| {
            let (name, version, description) = plugin_display_fields(report);
            let status = match &report.result {
                Ok(_) => "loaded".to_owned(),
                Err(error) => format!("error: {}", first_line(&error.to_string())),
            };
            json!({
                "name": name,
                "version": version,
                "status": status,
                "description": description,
            })
        })
        .collect()
}

fn plugin_display_fields(report: &crate::core::PluginLoadReport) -> (String, String, String) {
    match report.manifest.as_ref() {
        Some(manifest) => (
            manifest.name.clone(),
            manifest.version.clone(),
            manifest.description.clone().unwrap_or_default(),
        ),
        None => match report.result.as_ref() {
            Ok(info) => (info.name.clone(), "-".to_owned(), info.description.clone()),
            Err(_) => (
                report
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| report.path.display().to_string()),
                "-".to_owned(),
                String::new(),
            ),
        },
    }
}

fn first_line(text: &str) -> String {
    let mut lines = text.lines();
    let head = lines.next().unwrap_or("").trim_end().to_owned();
    if lines.next().is_some() {
        format!("{head} ...")
    } else {
        head
    }
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

fn approval_preview_for(call: &ToolCall, cwd: &Path) -> Option<AppApprovalPreview> {
    match call.name.as_str() {
        "apply_patch" => approval_preview_for_apply_patch(call),
        "write_file" => approval_preview_for_write_file(call, cwd),
        "shell" => approval_preview_for_shell(call, cwd),
        _ => None,
    }
}

fn approval_preview_for_apply_patch(call: &ToolCall) -> Option<AppApprovalPreview> {
    let patch = call
        .args
        .get("patch")
        .and_then(Value::as_str)
        .or_else(|| call.args.get("input").and_then(Value::as_str))?;
    let affected_files = affected_files_from_internal_patch(patch);
    let summary = if affected_files.is_empty() {
        "Apply workspace patch".to_owned()
    } else if affected_files.len() == 1 {
        format!("Patch {}", affected_files[0])
    } else {
        format!("Patch {} files", affected_files.len())
    };

    Some(
        AppApprovalPreview::new("patch", "Patch preview", summary)
            .with_affected_files(affected_files)
            .with_body(truncate_preview_body(patch), "diff")
            .with_metadata(json!({ "format": "proteus_internal_patch" })),
    )
}

fn approval_preview_for_write_file(call: &ToolCall, cwd: &Path) -> Option<AppApprovalPreview> {
    let path = call.args.get("path").and_then(Value::as_str)?;
    let content = call.args.get("content").and_then(Value::as_str)?;
    let target = preview_target_path(cwd, path);
    let existing_content = target
        .as_ref()
        .and_then(|target| existing_preview_content(cwd, target));
    let operation = match (&target, &existing_content) {
        (_, Some(_)) => "overwrite",
        (Some(_), None) => "create",
        (None, None) => "write",
    };
    let summary = match operation {
        "overwrite" => format!("Overwrite {path} ({} bytes)", content.len()),
        "create" => format!("Create {path} ({} bytes)", content.len()),
        _ => format!("Write {path} ({} bytes)", content.len()),
    };
    let (body, language) = match existing_content {
        Some(existing) => (simple_line_diff(path, &existing, content), "diff"),
        None => (content.to_owned(), "text"),
    };

    Some(
        AppApprovalPreview::new("write_file", "File write preview", summary)
            .with_affected_files(vec![path.to_owned()])
            .with_body(truncate_preview_body(&body), language)
            .with_metadata(json!({
                "operation": operation,
                "path": path,
                "target": target.as_ref().map(|target| target.display().to_string()),
                "workspace_scoped": target.is_some(),
                "bytes": content.len(),
            })),
    )
}

fn approval_preview_for_shell(call: &ToolCall, cwd: &Path) -> Option<AppApprovalPreview> {
    let command = call.args.get("command").and_then(Value::as_str)?;
    Some(
        AppApprovalPreview::new(
            "command",
            "Command preview",
            format!("Run shell command in {}", cwd.display()),
        )
        .with_body(truncate_preview_body(command), "shell")
        .with_metadata(json!({
            "cwd": cwd.display().to_string(),
            "cache_scope": "exact_command",
        })),
    )
}

fn preview_target_path(cwd: &Path, path: &str) -> Option<PathBuf> {
    let base = std::fs::canonicalize(cwd).ok()?;
    let path = Path::new(path);
    let relative = if path.is_absolute() {
        path.strip_prefix(&base).ok()?
    } else {
        path
    };
    Some(base.join(safe_preview_relative_path(relative)?))
}

fn safe_preview_relative_path(path: &Path) -> Option<PathBuf> {
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if safe.as_os_str().is_empty() {
        None
    } else {
        Some(safe)
    }
}

fn existing_preview_content(cwd: &Path, target: &Path) -> Option<String> {
    let base = std::fs::canonicalize(cwd).ok()?;
    let metadata = std::fs::symlink_metadata(target).ok()?;
    if metadata.file_type().is_symlink() {
        return None;
    }
    let canonical_target = std::fs::canonicalize(target).ok()?;
    if !canonical_target.starts_with(base) {
        return None;
    }
    std::fs::read_to_string(canonical_target).ok()
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn affected_files_from_internal_patch(patch: &str) -> Vec<String> {
    let mut files = BTreeSet::new();
    for line in patch.lines() {
        for prefix in [
            "*** Add File:",
            "*** Update File:",
            "*** Delete File:",
            "*** Move to:",
        ] {
            if let Some(path) = line.strip_prefix(prefix) {
                let path = path.trim();
                if !path.is_empty() {
                    files.insert(path.to_owned());
                }
            }
        }
    }
    files.into_iter().collect()
}

fn simple_line_diff(path: &str, old: &str, new: &str) -> String {
    if old == new {
        return format!("No content change for {path}");
    }

    let old_lines = old.lines().collect::<Vec<_>>();
    let new_lines = new.lines().collect::<Vec<_>>();
    let mut diff = format!("--- {path}\n+++ {path}\n@@\n");
    for index in 0..old_lines.len().max(new_lines.len()) {
        match (old_lines.get(index), new_lines.get(index)) {
            (Some(old), Some(new)) if old == new => {
                diff.push(' ');
                diff.push_str(old);
                diff.push('\n');
            }
            (Some(old), Some(new)) => {
                diff.push('-');
                diff.push_str(old);
                diff.push('\n');
                diff.push('+');
                diff.push_str(new);
                diff.push('\n');
            }
            (Some(old), None) => {
                diff.push('-');
                diff.push_str(old);
                diff.push('\n');
            }
            (None, Some(new)) => {
                diff.push('+');
                diff.push_str(new);
                diff.push('\n');
            }
            (None, None) => {}
        }
    }
    diff
}

fn truncate_preview_body(body: &str) -> String {
    if body.len() <= APPROVAL_PREVIEW_BODY_LIMIT {
        return body.to_owned();
    }

    let end = body
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= APPROVAL_PREVIEW_BODY_LIMIT)
        .last()
        .unwrap_or(0);
    format!(
        "{}\n\n[approval preview truncated to {} bytes]",
        &body[..end],
        APPROVAL_PREVIEW_BODY_LIMIT
    )
}

#[cfg(test)]
mod tests;
