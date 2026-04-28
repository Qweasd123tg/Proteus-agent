use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use tokio::sync::Mutex;

use crate::{
    contracts::{ApprovalTransport, EventEmitter, EventSink, MemoryPolicyInput},
    core::{AppConfig, BuiltinRegistry, JsonlEventStore, SessionStore},
    domain::{
        AgentOutput, AgentTask, Event, EventContext, PermissionMode, SessionId, ThreadId,
        new_session_id, new_thread_id, new_turn_id,
    },
    model_standard::CanonicalMessage,
    modules::HeadlessApprovalTransport,
};

pub struct AgentRuntime {
    cwd: PathBuf,
    session_id: SessionId,
    thread_id: ThreadId,
    registry: BuiltinRegistry,
    events: Arc<EventEmitter>,
    approval: Arc<dyn ApprovalTransport>,
    permission_mode: PermissionMode,
    run_lock: Mutex<()>,
    session_started: Mutex<bool>,
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
        let _run_guard = self.run_lock.lock().await;
        self.ensure_session_started().await?;
        let turn_id = new_turn_id();
        let event_context = EventContext {
            session_id: self.session_id,
            thread_id: self.thread_id,
            turn_id: Some(turn_id),
        };
        self.events
            .emit(
                event_context,
                Event::TurnStarted {
                    session_id: self.session_id,
                    thread_id: self.thread_id,
                    turn_id,
                },
            )
            .await?;
        let task = AgentTask {
            text,
            cwd: self.cwd.clone(),
        };
        let runtime_context = self.registry.runtime_context(
            self.session_id,
            self.thread_id,
            turn_id,
            self.events.clone(),
            self.approval.clone(),
            self.permission_mode,
        );
        let history = self.history.lock().await.clone();
        let previous_history_len = history.len();
        let workflow_output = self
            .registry
            .workflow
            .run(task.clone(), history, runtime_context)
            .await?;
        anyhow::ensure!(
            workflow_output.messages.len() >= previous_history_len,
            "workflow returned fewer messages than it received: output {}, input {}",
            workflow_output.messages.len(),
            previous_history_len
        );
        let new_messages = &workflow_output.messages[previous_history_len..];
        let memory_output = self
            .registry
            .memory_policy
            .after_turn(
                MemoryPolicyInput {
                    task: &task,
                    output: &workflow_output.output,
                    new_messages,
                },
                self.registry.memory.as_ref(),
            )
            .await?;
        for kind in memory_output.written_kinds {
            self.events
                .emit(
                    EventContext {
                        session_id: self.session_id,
                        thread_id: self.thread_id,
                        turn_id: Some(turn_id),
                    },
                    Event::MemoryWritten { kind },
                )
                .await?;
        }
        let mut history = self.history.lock().await;
        if let Some(session_store) = &self.session_store {
            session_store
                .append_messages(&workflow_output.messages[previous_history_len..])
                .await?;
        }
        *history = workflow_output.messages;
        Ok(workflow_output.output)
    }

    async fn ensure_session_started(&self) -> Result<()> {
        let mut started = self.session_started.lock().await;
        if *started {
            return Ok(());
        }

        self.events
            .emit(
                EventContext {
                    session_id: self.session_id,
                    thread_id: self.thread_id,
                    turn_id: None,
                },
                Event::SessionStarted {
                    session_id: self.session_id,
                    cwd: self.cwd.clone(),
                },
            )
            .await?;
        *started = true;
        Ok(())
    }

    pub async fn render(&self, output: &AgentOutput) -> Result<String> {
        self.registry.renderer.render(output).await
    }

    pub async fn clear_history(&self) -> Result<()> {
        let _run_guard = self.run_lock.lock().await;
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
        let event_sink: Arc<dyn EventSink> = event_sink
            .unwrap_or_else(|| Arc::new(JsonlEventStore::new(cwd.join(&config.event_log.path))));
        let events = Arc::new(EventEmitter::new(event_sink));
        let approval: Arc<dyn ApprovalTransport> =
            approval.unwrap_or_else(|| Arc::new(HeadlessApprovalTransport));
        let session_id = new_session_id();
        let thread_id = new_thread_id();
        let session_store = config_path
            .as_deref()
            .map(config_store_root)
            .map(|config_dir| SessionStore::new(&config_dir, &cwd, session_id));

        Ok(AgentRuntime {
            cwd,
            session_id,
            thread_id,
            registry,
            events,
            approval,
            permission_mode,
            run_lock: Mutex::new(()),
            session_started: Mutex::new(false),
            history: Mutex::new(Vec::new()),
            session_store,
        })
    }
}

fn config_store_root(path: &std::path::Path) -> PathBuf {
    if path.is_dir() {
        return path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf());
    }

    path.parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use async_trait::async_trait;

    use super::*;
    use crate::{
        contracts::{RuntimeContext, Workflow, WorkflowOutput},
        domain::{AgentOutput, AgentTask},
        model_standard::{CanonicalMessage, MessageRole},
    };

    struct ShortHistoryWorkflow;

    #[async_trait]
    impl Workflow for ShortHistoryWorkflow {
        async fn run(
            &self,
            _task: AgentTask,
            _history: Vec<CanonicalMessage>,
            _ctx: RuntimeContext,
        ) -> Result<WorkflowOutput> {
            Ok(WorkflowOutput {
                output: AgentOutput {
                    text: "bad workflow".to_owned(),
                    metadata: serde_json::Value::Null,
                },
                messages: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn run_errors_when_workflow_drops_existing_history() {
        let cwd = tempfile::tempdir().expect("temp dir");
        let mut runtime = AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
            .build()
            .expect("runtime");

        runtime.registry.workflow = Arc::new(ShortHistoryWorkflow);
        runtime
            .history
            .lock()
            .await
            .push(CanonicalMessage::text(MessageRole::User, "previous"));

        let error = runtime
            .run("current".to_owned())
            .await
            .expect_err("short workflow history must error");

        assert!(
            error
                .to_string()
                .contains("workflow returned fewer messages than it received")
        );
    }
}
