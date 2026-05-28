use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

use crate::{
    contracts::{
        ApprovalCacheScope, ApprovalResponse, CancellationToken, EventSink, FilteredEventSink,
        ToolSource, UserInputResponse, is_streaming_delta,
    },
    core::{
        AgentRuntime, AppConfig, BroadcastEventSink, BuiltinModuleCatalog,
        ChannelApprovalTransport, ChannelUserInputTransport, FanoutEventSink, JsonlEventStore,
        PendingApproval, PendingUserInput, normalize_session_dir_path, session_id_from_session_dir,
    },
    domain::{AgentOutput, PermissionMode, new_thread_id},
};

pub mod http;
pub mod protocol;
pub mod stdio;

// Wire protocol вынесен в proteus-contracts чтобы клиенты depend на него
// без зависимости на ядро. Здесь просто re-export для обратной
// совместимости внутри proteus-core.
pub use proteus_contracts::app_protocol::{
    AppApprovalId, AppApprovalRequest, AppServerEvent, AppUserInputRequestId, StdioOutput,
    StdioRequest,
};

type PendingApprovalResponders =
    Arc<Mutex<HashMap<AppApprovalId, tokio::sync::oneshot::Sender<ApprovalResponse>>>>;
type PendingUserInputResponders =
    Arc<Mutex<HashMap<AppUserInputRequestId, tokio::sync::oneshot::Sender<UserInputResponse>>>>;

#[derive(Clone)]
pub struct AppServerHandle {
    runtime: Arc<AgentRuntime>,
    config: Arc<AppConfig>,
    config_path: Option<PathBuf>,
    cwd: PathBuf,
    plugin_reports: Arc<Vec<crate::core::PluginLoadReport>>,
    events: broadcast::Sender<AppServerEvent>,
    pending_approvals: PendingApprovalResponders,
    pending_user_inputs: PendingUserInputResponders,
}

impl AppServerHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<AppServerEvent> {
        self.events.subscribe()
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
                    output: output.clone(),
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

    pub async fn config_summary(&self) -> Value {
        let mode = self.permission_mode().await;
        let tools = self.runtime.tool_entries();
        let config_files = config_files(self.config_path.as_deref());
        let model = self.config.active_model_config().ok();
        json!({
            "display_text": render_config_summary(
                &self.config,
                self.config_path.as_deref(),
                &self.cwd,
                mode,
                &tools,
                &self.plugin_reports,
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
            "profile": self.config.profile.name,
            "model": model.as_ref().map(|model| json!({
                "provider": model.provider,
                "name": model.model,
                "label": format!("{}/{}", model.provider, model.model),
            })),
            "permission_mode": format!("{mode:?}"),
            "modules": module_summary(&self.config),
            "tools_enabled": self.config.tools.enabled,
            "registered_tools": tools
                .iter()
                .map(|(source, spec)| json!({
                    "name": spec.name,
                    "source": source.label(),
                    "safety": format!("{:?}", spec.safety),
                    "description": spec.description,
                }))
                .collect::<Vec<_>>(),
            "plugins": plugin_summary(&self.plugin_reports),
        })
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

        let config_snapshot = Arc::new(config.clone());
        let config_path_snapshot = config_path.map(Path::to_path_buf);
        let cwd_snapshot = cwd.clone();
        let (module_catalog, plugin_reports) = match module_catalog {
            Some(catalog) => (Some(catalog), Vec::new()),
            None => {
                let mut catalog = BuiltinModuleCatalog::new();
                let reports = crate::core::default_plugins_dir()
                    .map(|plugins_dir| {
                        crate::core::load_plugins_from_dir(&plugins_dir, &mut catalog)
                    })
                    .unwrap_or_default();
                (Some(catalog), reports)
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
            plugin_reports: Arc::new(plugin_reports),
            events,
            pending_approvals,
            pending_user_inputs,
        })
    }
}

fn render_config_summary(
    config: &AppConfig,
    config_path: Option<&Path>,
    cwd: &Path,
    mode: PermissionMode,
    tools: &[(ToolSource, crate::domain::ToolSpec)],
    plugin_reports: &[crate::core::PluginLoadReport],
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
                    let _ = events.send(AppServerEvent::Runtime { envelope });
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
            pending_approvals
                .lock()
                .await
                .insert(approval_id.clone(), responder);
            let app_request = AppApprovalRequest::new(
                approval_id.clone(),
                request.call,
                request.cwd,
                request.reason,
                request.tool_spec,
            );
            if events
                .send(AppServerEvent::ApprovalRequested {
                    request: app_request,
                })
                .is_err()
                && let Some(responder) = pending_approvals.lock().await.remove(&approval_id)
            {
                let _ = responder.send(ApprovalResponse::deny(
                    "approval request could not be delivered to any app-server client",
                ));
                let _ = events.send(AppServerEvent::ApprovalResolved {
                    approval_id,
                    approved: false,
                });
                continue;
            }

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
        let responder = pending_approvals.lock().await.remove(&approval_id);
        if let Some(responder) = responder {
            let timeout_ms = approval_timeout.as_millis() as u64;
            let _ = responder.send(ApprovalResponse::deny(format!(
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
            pending_user_inputs
                .lock()
                .await
                .insert(request_id.clone(), responder);
            if events
                .send(AppServerEvent::UserInputRequested { request })
                .is_err()
                && let Some(responder) = pending_user_inputs.lock().await.remove(&request_id)
            {
                let _ = responder.send(UserInputResponse::empty());
                let _ = events.send(AppServerEvent::UserInputResolved { request_id });
                continue;
            }

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
        let responder = pending_user_inputs.lock().await.remove(&request_id);
        if let Some(responder) = responder {
            let _ = responder.send(UserInputResponse::empty());
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
    for (approval_id, responder) in pending {
        let _ = responder.send(ApprovalResponse::deny(note.clone()));
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
    for (request_id, responder) in pending {
        let _ = responder.send(UserInputResponse::empty());
        let _ = events.send(AppServerEvent::UserInputResolved { request_id });
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

    use coding_workflow::CodingPlanExecuteReviewWorkflow;
    use context_pack::SimpleContextBuilderPlugin;
    use policy_pack::AskWritePolicyPlugin;
    use proteus_contracts::{
        abi_stable::sabi_trait::TD_Opaque,
        contracts::Renderer_TO,
        plugin::{PluginApprovalPolicy_TO, PluginContextBuilder_TO, PluginWorkflow_TO},
    };
    use renderer_pack::PlainRendererPlugin;
    use tokio::sync::{Mutex, broadcast, mpsc, oneshot};

    use super::*;
    use crate::{
        contracts::{
            ApprovalRequest, UserInputQuestion, UserInputQuestionOption, UserInputRequest,
        },
        core::{PendingApproval, PendingUserInput},
        domain::{Event, PermissionMode, ToolCall, new_call_id},
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

    #[tokio::test]
    async fn app_server_updates_permission_mode_without_restart() {
        let cwd = tempfile::tempdir().expect("cwd");
        let mut config = AppConfig::default();
        config.permissions.mode = PermissionMode::Normal;
        let server = AgentAppServer::launch_with_module_catalog(
            config,
            cwd.path().to_path_buf(),
            None,
            test_catalog(),
        )
        .expect("app server");

        assert_eq!(server.permission_mode().await, PermissionMode::Normal);

        server.set_permission_mode(PermissionMode::Plan).await;

        assert_eq!(server.permission_mode().await, PermissionMode::Plan);
    }

    #[tokio::test]
    async fn approval_forwarder_denies_when_no_client_can_receive_request() {
        let (approval_tx, approval_rx) = mpsc::channel(1);
        let (events, _) = broadcast::channel(1);
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
        spawn_approval_forwarder(
            approval_rx,
            events,
            pending_approvals.clone(),
            Duration::from_secs(60),
        );

        let (responder, response_rx) = oneshot::channel();
        approval_tx
            .send(PendingApproval {
                request: ApprovalRequest::new(
                    ToolCall::new(new_call_id(), "write_file", serde_json::json!({})),
                    PathBuf::from("."),
                    "test approval",
                    None,
                ),
                responder,
            })
            .await
            .unwrap();

        let response = tokio::time::timeout(Duration::from_secs(1), response_rx)
            .await
            .expect("approval response should not hang")
            .expect("approval responder should send denial");

        assert!(!response.approved);
        assert!(
            response
                .note
                .as_deref()
                .is_some_and(|note| note.contains("could not be delivered"))
        );
        assert!(pending_approvals.lock().await.is_empty());
    }

    #[tokio::test]
    async fn approval_forwarder_denies_when_client_does_not_answer_before_timeout() {
        let (approval_tx, approval_rx) = mpsc::channel(1);
        let (events, _) = broadcast::channel(8);
        let mut event_rx = events.subscribe();
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
        spawn_approval_forwarder(
            approval_rx,
            events,
            pending_approvals.clone(),
            Duration::from_millis(20),
        );

        let (responder, response_rx) = oneshot::channel();
        approval_tx
            .send(PendingApproval {
                request: ApprovalRequest::new(
                    ToolCall::new(new_call_id(), "write_file", serde_json::json!({})),
                    PathBuf::from("."),
                    "test approval",
                    None,
                ),
                responder,
            })
            .await
            .unwrap();

        let request_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("approval request event should arrive")
            .expect("event stream should stay open");
        let approval_id = match request_event {
            AppServerEvent::ApprovalRequested { request } => request.approval_id,
            other => panic!("expected approval request, got {other:?}"),
        };

        let response = tokio::time::timeout(Duration::from_secs(1), response_rx)
            .await
            .expect("approval response should not hang")
            .expect("approval responder should send denial");

        assert!(!response.approved);
        assert!(
            response
                .note
                .as_deref()
                .is_some_and(|note| note.contains("timed out"))
        );
        assert!(pending_approvals.lock().await.is_empty());

        let resolved_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("approval resolved event should arrive")
            .expect("event stream should stay open");
        assert!(matches!(
            resolved_event,
            AppServerEvent::ApprovalResolved {
                approval_id: id,
                approved: false,
            } if id == approval_id
        ));
    }

    #[tokio::test]
    async fn approval_forwarder_waits_without_timeout_when_timeout_is_zero() {
        let (approval_tx, approval_rx) = mpsc::channel(1);
        let (events, _) = broadcast::channel(8);
        let mut event_rx = events.subscribe();
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
        spawn_approval_forwarder(
            approval_rx,
            events,
            pending_approvals.clone(),
            Duration::ZERO,
        );

        let (responder, mut response_rx) = oneshot::channel();
        approval_tx
            .send(PendingApproval {
                request: ApprovalRequest::new(
                    ToolCall::new(new_call_id(), "write_file", serde_json::json!({})),
                    PathBuf::from("."),
                    "test approval",
                    None,
                ),
                responder,
            })
            .await
            .unwrap();

        let request_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("approval request event should arrive")
            .expect("event stream should stay open");
        let approval_id = match request_event {
            AppServerEvent::ApprovalRequested { request } => request.approval_id,
            other => panic!("expected approval request, got {other:?}"),
        };

        tokio::time::sleep(Duration::from_millis(30)).await;

        assert!(pending_approvals.lock().await.contains_key(&approval_id));
        assert!(response_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn user_input_forwarder_waits_without_timeout_when_timeout_is_zero() {
        let (user_input_tx, user_input_rx) = mpsc::channel(1);
        let (events, _) = broadcast::channel(8);
        let mut event_rx = events.subscribe();
        let pending_user_inputs = Arc::new(Mutex::new(HashMap::new()));
        spawn_user_input_forwarder(
            user_input_rx,
            events,
            pending_user_inputs.clone(),
            Duration::ZERO,
        );

        let request_id = "question-1".to_owned();
        let (responder, mut response_rx) = oneshot::channel();
        user_input_tx
            .send(PendingUserInput {
                request: UserInputRequest::new(
                    request_id.clone(),
                    PathBuf::from("."),
                    vec![UserInputQuestion::new(
                        "scope",
                        "Scope",
                        "Which scope?",
                        vec![UserInputQuestionOption::new("Small", "Small scope")],
                    )],
                ),
                responder,
            })
            .await
            .unwrap();

        let request_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("user input request event should arrive")
            .expect("event stream should stay open");
        assert!(matches!(
            request_event,
            AppServerEvent::UserInputRequested { request } if request.request_id == request_id
        ));

        tokio::time::sleep(Duration::from_millis(30)).await;

        assert!(pending_user_inputs.lock().await.contains_key(&request_id));
        assert!(response_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn shutdown_denies_pending_approvals() {
        let (events, _) = broadcast::channel(8);
        let mut event_rx = events.subscribe();
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
        let (responder, response_rx) = oneshot::channel();
        let approval_id = "approval-1".to_owned();
        pending_approvals
            .lock()
            .await
            .insert(approval_id.clone(), responder);

        deny_pending_approvals(
            pending_approvals.clone(),
            events,
            "app-server shutting down".to_owned(),
        )
        .await;

        let response = response_rx
            .await
            .expect("shutdown should send approval response");
        assert!(!response.approved);
        assert_eq!(response.note.as_deref(), Some("app-server shutting down"));
        assert!(pending_approvals.lock().await.is_empty());

        let resolved_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("approval resolved event should arrive")
            .expect("event stream should stay open");
        assert!(matches!(
            resolved_event,
            AppServerEvent::ApprovalResolved {
                approval_id: id,
                approved: false,
            } if id == approval_id
        ));
    }

    #[tokio::test]
    async fn shutdown_resolves_pending_user_inputs() {
        let (events, _) = broadcast::channel(8);
        let mut event_rx = events.subscribe();
        let pending_user_inputs = Arc::new(Mutex::new(HashMap::new()));
        let (responder, response_rx) = oneshot::channel();
        let request_id = "input-1".to_owned();
        pending_user_inputs
            .lock()
            .await
            .insert(request_id.clone(), responder);

        deny_pending_user_inputs(
            pending_user_inputs.clone(),
            events,
            "app-server shutting down".to_owned(),
        )
        .await;

        let response = response_rx
            .await
            .expect("shutdown should send user input response");
        assert!(response.answers.is_empty());
        assert!(pending_user_inputs.lock().await.is_empty());

        let resolved_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("user input resolved event should arrive")
            .expect("event stream should stay open");
        assert!(matches!(
            resolved_event,
            AppServerEvent::UserInputResolved { request_id: id } if id == request_id
        ));
    }

    #[tokio::test]
    async fn cancel_pending_approvals_denies_pending_requests() {
        let cwd = tempfile::tempdir().expect("cwd");
        let mut config = AppConfig::default();
        config.modules.patch = "null".to_owned();
        let handle = AgentAppServer::launch_with_module_catalog(
            config,
            cwd.path().to_path_buf(),
            None,
            test_catalog(),
        )
        .expect("app server");
        let mut event_rx = handle.subscribe();
        let (responder, response_rx) = oneshot::channel();
        let approval_id = "approval-cancel".to_owned();
        handle
            .pending_approvals
            .lock()
            .await
            .insert(approval_id.clone(), responder);

        handle
            .cancel_pending_approvals("turn canceled by client".to_owned())
            .await;

        let response = response_rx
            .await
            .expect("cancel should send approval response");
        assert!(!response.approved);
        assert_eq!(response.note.as_deref(), Some("turn canceled by client"));
        assert!(handle.pending_approvals.lock().await.is_empty());

        let resolved_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("approval resolved event should arrive")
            .expect("event stream should stay open");
        assert!(matches!(
            resolved_event,
            AppServerEvent::ApprovalResolved {
                approval_id: id,
                approved: false,
            } if id == approval_id
        ));

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn cancel_pending_user_inputs_resolves_pending_requests() {
        let cwd = tempfile::tempdir().expect("cwd");
        let mut config = AppConfig::default();
        config.modules.patch = "null".to_owned();
        let handle = AgentAppServer::launch_with_module_catalog(
            config,
            cwd.path().to_path_buf(),
            None,
            test_catalog(),
        )
        .expect("app server");
        let mut event_rx = handle.subscribe();
        let (responder, response_rx) = oneshot::channel();
        let request_id = "input-cancel".to_owned();
        handle
            .pending_user_inputs
            .lock()
            .await
            .insert(request_id.clone(), responder);

        handle
            .cancel_pending_user_inputs("turn canceled by client".to_owned())
            .await;

        let response = response_rx
            .await
            .expect("cancel should send user input response");
        assert!(response.answers.is_empty());
        assert!(handle.pending_user_inputs.lock().await.is_empty());

        let resolved_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("user input resolved event should arrive")
            .expect("event stream should stay open");
        assert!(matches!(
            resolved_event,
            AppServerEvent::UserInputResolved { request_id: id } if id == request_id
        ));

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn zero_timeout_pending_user_input_resolves_on_shutdown() {
        let (user_input_tx, user_input_rx) = mpsc::channel(1);
        let (events, _) = broadcast::channel(8);
        let mut event_rx = events.subscribe();
        let pending_user_inputs = Arc::new(Mutex::new(HashMap::new()));
        spawn_user_input_forwarder(
            user_input_rx,
            events.clone(),
            pending_user_inputs.clone(),
            Duration::ZERO,
        );

        let request_id = "question-shutdown".to_owned();
        let (responder, response_rx) = oneshot::channel();
        user_input_tx
            .send(PendingUserInput {
                request: UserInputRequest::new(
                    request_id.clone(),
                    PathBuf::from("."),
                    vec![UserInputQuestion::new(
                        "scope",
                        "Scope",
                        "Which scope?",
                        vec![UserInputQuestionOption::new("Small", "Small scope")],
                    )],
                ),
                responder,
            })
            .await
            .unwrap();

        let request_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("user input request event should arrive")
            .expect("event stream should stay open");
        assert!(matches!(
            request_event,
            AppServerEvent::UserInputRequested { request } if request.request_id == request_id
        ));

        deny_pending_user_inputs(
            pending_user_inputs.clone(),
            events,
            "app-server shutting down".to_owned(),
        )
        .await;

        let response = tokio::time::timeout(Duration::from_secs(1), response_rx)
            .await
            .expect("user input response should not hang")
            .expect("user input responder should send empty response");
        assert!(response.answers.is_empty());
        assert!(pending_user_inputs.lock().await.is_empty());
    }

    #[tokio::test]
    async fn app_server_forwards_streaming_text_deltas_before_turn_output() {
        let cwd = tempfile::tempdir().expect("cwd");
        let mut config = AppConfig::default();
        config.modules.workflow = "coding.plan_execute_review".to_owned();
        config.modules.context = "simple".to_owned();
        config.modules.policy = "ask_write".to_owned();
        config.modules.renderer = "plain".to_owned();
        config.modules.patch = "null".to_owned();

        let handle = AgentAppServer::launch_with_module_catalog(
            config,
            cwd.path().to_path_buf(),
            None,
            test_catalog(),
        )
        .expect("app server");
        let mut event_rx = handle.subscribe();
        let send_handle = handle.clone();
        let turn = tokio::spawn(async move {
            send_handle
                .send_user_message("stream this".to_owned())
                .await
                .expect("turn output")
        });

        let mut saw_delta = false;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
                .await
                .expect("event should arrive")
                .expect("event stream should stay open");
            match event {
                AppServerEvent::Runtime { envelope } => {
                    if matches!(envelope.event, Event::AssistantTextDelta { .. }) {
                        saw_delta = true;
                    }
                }
                AppServerEvent::TurnOutput { .. } => break,
                AppServerEvent::Error { message } => {
                    panic!("unexpected app-server error: {message}")
                }
                _ => {}
            }
        }

        let output = turn.await.expect("turn task");
        assert!(
            saw_delta,
            "expected at least one text delta before TurnOutput"
        );
        assert!(output.text.contains("Fake final answer"));
        handle.shutdown().await;
    }
}
