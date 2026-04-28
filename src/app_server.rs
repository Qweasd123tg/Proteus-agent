use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

use crate::{
    contracts::{ApprovalResponse, EventSink},
    core::{AgentRuntime, AppConfig, BroadcastEventSink, FanoutEventSink, JsonlEventStore},
    domain::{AgentOutput, Event, ToolCall, ToolSpec},
    modules::{ChannelApprovalTransport, PendingApproval},
};

pub mod protocol;
pub mod stdio;

pub type AppApprovalId = String;

/// Protocol event exposed by the local app-server boundary. UI clients should
/// depend on this stream instead of depending directly on runtime internals.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppServerEvent {
    Runtime {
        event: Event,
    },
    UserMessageSubmitted {
        text: String,
    },
    TurnOutput {
        output: AgentOutput,
    },
    ApprovalRequested {
        request: AppApprovalRequest,
    },
    ApprovalResolved {
        approval_id: AppApprovalId,
        approved: bool,
    },
    Error {
        message: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppApprovalRequest {
    pub approval_id: AppApprovalId,
    pub call: ToolCall,
    pub cwd: PathBuf,
    pub reason: String,
    pub tool_spec: Option<ToolSpec>,
}

#[derive(Clone)]
pub struct AppServerHandle {
    runtime: Arc<AgentRuntime>,
    events: broadcast::Sender<AppServerEvent>,
    pending_approvals:
        Arc<Mutex<HashMap<AppApprovalId, tokio::sync::oneshot::Sender<ApprovalResponse>>>>,
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
    ) -> Result<()> {
        let responder = self
            .pending_approvals
            .lock()
            .await
            .remove(approval_id)
            .ok_or_else(|| anyhow!("unknown approval id: {approval_id}"))?;
        responder
            .send(ApprovalResponse { approved, note })
            .map_err(|_| anyhow!("approval response channel dropped"))?;
        let _ = self.events.send(AppServerEvent::ApprovalResolved {
            approval_id: approval_id.to_owned(),
            approved,
        });
        Ok(())
    }

    pub fn shutdown(&self) {
        let _ = self.events.send(AppServerEvent::Shutdown);
    }
}

pub struct AgentAppServer;

impl AgentAppServer {
    pub fn launch(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&Path>,
    ) -> Result<AppServerHandle> {
        let core_broadcast = Arc::new(BroadcastEventSink::new(1024));
        let jsonl = Arc::new(JsonlEventStore::new(cwd.join(&config.event_log.path)));
        let event_sink: Arc<dyn EventSink> =
            Arc::new(FanoutEventSink::new(vec![jsonl, core_broadcast.clone()]));

        let (approval_transport, approval_rx) = ChannelApprovalTransport::new(32);
        let runtime = Arc::new(
            AgentRuntime::builder(config, cwd)
                .with_config_path(config_path)
                .with_event_sink(event_sink)
                .with_approval(Arc::new(approval_transport))
                .build()?,
        );
        let (events, _) = broadcast::channel(1024);
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));

        spawn_runtime_event_forwarder(core_broadcast, events.clone());
        spawn_approval_forwarder(approval_rx, events.clone(), pending_approvals.clone());

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
    pending_approvals: Arc<
        Mutex<HashMap<AppApprovalId, tokio::sync::oneshot::Sender<ApprovalResponse>>>,
    >,
) {
    tokio::spawn(async move {
        while let Some(PendingApproval { request, responder }) = approval_rx.recv().await {
            let approval_id = Uuid::new_v4().to_string();
            pending_approvals
                .lock()
                .await
                .insert(approval_id.clone(), responder);
            let app_request = AppApprovalRequest {
                approval_id,
                call: request.call,
                cwd: request.cwd,
                reason: request.reason,
                tool_spec: request.tool_spec,
            };
            let _ = events.send(AppServerEvent::ApprovalRequested {
                request: app_request,
            });
        }
    });
}
