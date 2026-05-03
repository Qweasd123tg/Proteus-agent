use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, anyhow};
use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

use crate::{
    contracts::{
        ApprovalCacheScope, ApprovalResponse, EventSink, FilteredEventSink, is_streaming_delta,
    },
    core::{
        AgentRuntime, AppConfig, BroadcastEventSink, BuiltinModuleCatalog,
        ChannelApprovalTransport, FanoutEventSink, JsonlEventStore, PendingApproval,
        normalize_session_dir_path, session_id_from_session_dir,
    },
    domain::{AgentOutput, new_thread_id},
};

pub mod protocol;
pub mod stdio;

// Wire protocol вынесен в agent-contracts чтобы клиенты depend на него
// без зависимости на ядро. Здесь просто re-export для обратной
// совместимости внутри modular-agent.
pub use agent_contracts::app_protocol::{
    AppApprovalId, AppApprovalRequest, AppServerEvent, StdioOutput, StdioRequest,
};

type PendingApprovalResponders =
    Arc<Mutex<HashMap<AppApprovalId, tokio::sync::oneshot::Sender<ApprovalResponse>>>>;

#[derive(Clone)]
pub struct AppServerHandle {
    runtime: Arc<AgentRuntime>,
    events: broadcast::Sender<AppServerEvent>,
    pending_approvals: PendingApprovalResponders,
}

impl AppServerHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<AppServerEvent> {
        self.events.subscribe()
    }

    pub async fn send_user_message(&self, text: String) -> Result<AgentOutput> {
        let _ = self
            .events
            .send(AppServerEvent::UserMessageSubmitted { text: text.clone() });
        match self.runtime.run(text).await {
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

    pub async fn shutdown(&self) {
        deny_pending_approvals(
            self.pending_approvals.clone(),
            self.events.clone(),
            "app-server shutting down".to_owned(),
        )
        .await;
        let _ = self.events.send(AppServerEvent::Shutdown);
    }

    pub async fn cancel_pending_approvals(&self, note: String) {
        deny_pending_approvals(self.pending_approvals.clone(), self.events.clone(), note).await;
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
        cwd: PathBuf,
        config_path: Option<&Path>,
        module_catalog: Option<BuiltinModuleCatalog>,
        resume_session_dir: Option<PathBuf>,
    ) -> Result<AppServerHandle> {
        let core_broadcast = Arc::new(BroadcastEventSink::new(1024));
        let jsonl_raw: Arc<dyn EventSink> =
            Arc::new(JsonlEventStore::new(cwd.join(&config.event_log.path)));
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
        let mut builder = AgentRuntime::builder(config, cwd)
            .with_config_path(config_path)
            .with_event_sink(event_sink)
            .with_approval(Arc::new(approval_transport));
        if let Some(session_dir) = resume_session_dir {
            let session_dir = normalize_session_dir_path(session_dir)?;
            let session_id = session_id_from_session_dir(&session_dir)?;
            builder = builder.resume_from_session_dir(session_dir, session_id, new_thread_id());
        }
        if let Some(module_catalog) = module_catalog {
            builder = builder.with_module_catalog(module_catalog);
        }
        let runtime = Arc::new(builder.build()?);
        let (events, _) = broadcast::channel(1024);
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));

        spawn_runtime_event_forwarder(core_broadcast, events.clone());
        spawn_approval_forwarder(
            approval_rx,
            events.clone(),
            pending_approvals.clone(),
            approval_timeout,
        );

        Ok(AppServerHandle {
            runtime,
            events,
            pending_approvals,
        })
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
                Ok(event) => {
                    let _ = events.send(AppServerEvent::Runtime { event });
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

            spawn_approval_timeout(
                approval_id,
                pending_approvals.clone(),
                events.clone(),
                approval_timeout,
            );
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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

    use agent_contracts::{
        abi_stable::sabi_trait::TD_Opaque,
        contracts::Renderer_TO,
        plugin::{PluginApprovalPolicy_TO, PluginContextBuilder_TO, PluginWorkflow_TO},
    };
    use coding_workflow::CodingPlanExecuteReviewWorkflow;
    use context_pack::SimpleContextBuilderPlugin;
    use policy_pack::AskWritePolicyPlugin;
    use renderer_pack::PlainRendererPlugin;
    use tokio::sync::{Mutex, broadcast, mpsc, oneshot};

    use super::*;
    use crate::{
        contracts::ApprovalRequest,
        core::PendingApproval,
        domain::{ToolCall, new_call_id},
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
}
