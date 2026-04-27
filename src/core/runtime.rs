use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use tokio::sync::Mutex;

use crate::{
    contracts::{ApprovalTransport, EventSink},
    core::{AppConfig, BuiltinRegistry, JsonlEventStore, SessionStore},
    domain::{AgentOutput, AgentTask, Event, PermissionMode, new_session_id},
    model_standard::CanonicalMessage,
    modules::HeadlessApprovalTransport,
};

pub struct AgentRuntime {
    cwd: PathBuf,
    registry: BuiltinRegistry,
    event_sink: Arc<dyn EventSink>,
    approval: Arc<dyn ApprovalTransport>,
    permission_mode: PermissionMode,
    history: Mutex<Vec<CanonicalMessage>>,
    session_store: Option<SessionStore>,
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
            self.permission_mode,
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

/// Builder for `AgentRuntime`. Every slot has a sensible default
/// (headless approval, jsonl event log derived from the config, no session
/// persistence) so callers only override what they actually want to change.
pub struct AgentRuntimeBuilder {
    config: AppConfig,
    cwd: PathBuf,
    config_path: Option<PathBuf>,
    event_sink: Option<Arc<dyn EventSink>>,
    approval: Option<Arc<dyn ApprovalTransport>>,
}

impl AgentRuntimeBuilder {
    pub fn new(config: AppConfig, cwd: PathBuf) -> Self {
        Self {
            config,
            cwd,
            config_path: None,
            event_sink: None,
            approval: None,
        }
    }

    pub fn with_config_path(mut self, path: Option<&std::path::Path>) -> Self {
        self.config_path = path.map(|p| p.to_path_buf());
        self
    }

    pub fn with_event_sink(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    pub fn with_approval(mut self, approval: Arc<dyn ApprovalTransport>) -> Self {
        self.approval = Some(approval);
        self
    }

    pub fn build(self) -> Result<AgentRuntime> {
        let Self {
            config,
            cwd,
            config_path,
            event_sink,
            approval,
        } = self;

        let permission_mode = config.permissions.mode;
        let registry = BuiltinRegistry::from_config(&config, cwd.clone())?;
        let event_sink: Arc<dyn EventSink> = event_sink.unwrap_or_else(|| {
            Arc::new(JsonlEventStore::new(cwd.join(&config.event_log.path)))
        });
        let approval: Arc<dyn ApprovalTransport> =
            approval.unwrap_or_else(|| Arc::new(HeadlessApprovalTransport));
        let session_store = config_path
            .as_deref()
            .and_then(|path| path.parent())
            .map(|config_dir| SessionStore::new(config_dir, &cwd));

        Ok(AgentRuntime {
            cwd,
            registry,
            event_sink,
            approval,
            permission_mode,
            history: Mutex::new(Vec::new()),
            session_store,
        })
    }
}
