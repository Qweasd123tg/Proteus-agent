use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use tokio::sync::Mutex;

use crate::{
    contracts::{ApprovalTransport, EventSink},
    core::{AppConfig, BuiltinRegistry, JsonlEventStore, SessionStore},
    domain::{AgentOutput, AgentTask, Event, new_session_id},
    model_standard::CanonicalMessage,
    modules::HeadlessApprovalTransport,
};

pub struct AgentRuntime {
    cwd: PathBuf,
    registry: BuiltinRegistry,
    event_sink: Arc<dyn EventSink>,
    approval: Arc<dyn ApprovalTransport>,
    history: Mutex<Vec<CanonicalMessage>>,
    session_store: Option<SessionStore>,
}

impl AgentRuntime {
    pub fn new(config: AppConfig, cwd: PathBuf) -> Result<Self> {
        let config_path = AppConfig::default_user_config_path();
        Self::new_with_config_path(config, cwd, config_path.as_deref())
    }

    pub fn new_with_config_path(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&std::path::Path>,
    ) -> Result<Self> {
        Self::new_with_config_path_and_approval_transport(
            config,
            cwd,
            config_path,
            Arc::new(HeadlessApprovalTransport),
        )
    }

    pub fn new_with_config_path_and_approval_transport(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<&std::path::Path>,
        approval: Arc<dyn ApprovalTransport>,
    ) -> Result<Self> {
        let registry = BuiltinRegistry::from_config(&config, cwd.clone())?;
        let event_sink: Arc<dyn EventSink> =
            Arc::new(JsonlEventStore::new(cwd.join(&config.event_log.path)));
        let session_store = config_path
            .and_then(|path| path.parent())
            .map(|config_dir| SessionStore::new(config_dir, &cwd));
        Ok(Self {
            cwd,
            registry,
            event_sink,
            approval,
            history: Mutex::new(Vec::new()),
            session_store,
        })
    }

    pub fn with_event_sink(
        config: AppConfig,
        cwd: PathBuf,
        event_sink: Arc<dyn EventSink>,
    ) -> Result<Self> {
        Self::with_event_sink_and_approval_transport(
            config,
            cwd,
            event_sink,
            Arc::new(HeadlessApprovalTransport),
        )
    }

    pub fn with_event_sink_and_approval_transport(
        config: AppConfig,
        cwd: PathBuf,
        event_sink: Arc<dyn EventSink>,
        approval: Arc<dyn ApprovalTransport>,
    ) -> Result<Self> {
        let registry = BuiltinRegistry::from_config(&config, cwd.clone())?;
        Ok(Self {
            cwd,
            registry,
            event_sink,
            approval,
            history: Mutex::new(Vec::new()),
            session_store: None,
        })
    }

    pub async fn run(&self, text: String) -> Result<AgentOutput> {
        let session_id = new_session_id();
        self.event_sink
            .append(Event::SessionStarted {
                session_id,
                cwd: self.cwd.clone(),
            })
            .await?;
        let task = AgentTask {
            text,
            cwd: self.cwd.clone(),
        };
        let runtime_context = self.registry.runtime_context(
            session_id,
            self.event_sink.clone(),
            self.approval.clone(),
        );
        let history = self.history.lock().await.clone();
        let workflow_output = self
            .registry
            .workflow
            .run(task, history, runtime_context)
            .await?;
        let mut history = self.history.lock().await;
        if let Some(session_store) = &self.session_store {
            session_store
                .append_messages(&workflow_output.messages[history.len()..])
                .await?;
        }
        *history = workflow_output.messages;
        Ok(workflow_output.output)
    }

    pub async fn render(&self, output: &AgentOutput) -> Result<String> {
        self.registry.renderer.render(output).await
    }

    pub async fn clear_history(&self) -> Result<()> {
        self.history.lock().await.clear();
        if let Some(session_store) = &self.session_store {
            session_store.clear().await?;
        }
        Ok(())
    }

    pub async fn history_len(&self) -> usize {
        self.history.lock().await.len()
    }

    pub fn session_dir(&self) -> Option<&std::path::Path> {
        self.session_store.as_ref().map(|store| store.session_dir())
    }
}
