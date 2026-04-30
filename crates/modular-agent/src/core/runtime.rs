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
    modules::{CachedApprovalTransport, HeadlessApprovalTransport},
};

pub struct AgentRuntime {
    services: RuntimeServices,
    session: SessionState,
}

struct RuntimeServices {
    cwd: PathBuf,
    registry: BuiltinRegistry,
    events: Arc<EventEmitter>,
    approval: Arc<dyn ApprovalTransport>,
    permission_mode: PermissionMode,
}

struct SessionState {
    session_id: SessionId,
    thread_id: ThreadId,
    run_lock: Mutex<()>,
    session_started: Mutex<bool>,
    history: Mutex<Vec<CanonicalMessage>>,
    session_store: Option<SessionStore>,
}

impl SessionState {
    fn new(
        session_id: SessionId,
        thread_id: ThreadId,
        session_store: Option<SessionStore>,
        history: Vec<CanonicalMessage>,
        session_started: bool,
    ) -> Self {
        Self {
            session_id,
            thread_id,
            run_lock: Mutex::new(()),
            session_started: Mutex::new(session_started),
            history: Mutex::new(history),
            session_store,
        }
    }
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
        let _run_guard = self.session.run_lock.lock().await;
        self.ensure_session_started().await?;
        let turn_id = new_turn_id();
        let event_context = EventContext::new(
            self.session.session_id,
            self.session.thread_id,
            Some(turn_id),
        );
        self.services
            .events
            .emit(
                event_context,
                Event::TurnStarted {
                    session_id: self.session.session_id,
                    thread_id: self.session.thread_id,
                    turn_id,
                },
            )
            .await?;
        let task = AgentTask::new(text, self.services.cwd.clone());
        // Выставляем delta event context для ModelService, чтобы
        // streaming TextDelta/ToolArgsDelta/ReasoningDelta эмитились с
        // правильным envelope (session/thread/turn). Без этого дельты
        // тихо дропаются (штатное поведение без runtime).
        if let Some(service) = &self.services.registry.model_service {
            service.set_event_context(crate::modules::DeltaEventContext {
                emitter: Some(self.services.events.clone()),
                session_id: Some(self.session.session_id),
                thread_id: Some(self.session.thread_id),
                turn_id: Some(turn_id),
            });
        }
        let runtime_context = self.services.registry.runtime_context(
            self.session.session_id,
            self.session.thread_id,
            turn_id,
            self.services.events.clone(),
            self.services.approval.clone(),
            self.services.permission_mode,
        );
        let history = self.session.history.lock().await.clone();
        let previous_history_len = history.len();
        let workflow_output = self
            .services
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
            .services
            .registry
            .memory_policy
            .after_turn(
                MemoryPolicyInput {
                    task: &task,
                    output: &workflow_output.output,
                    new_messages,
                },
                self.services.registry.memory.as_ref(),
            )
            .await?;
        for kind in memory_output.written_kinds {
            self.services
                .events
                .emit(
                    EventContext::new(
                        self.session.session_id,
                        self.session.thread_id,
                        Some(turn_id),
                    ),
                    Event::MemoryWritten { kind },
                )
                .await?;
        }
        let mut history = self.session.history.lock().await;
        if let Some(session_store) = &self.session.session_store {
            session_store
                .append_messages(&workflow_output.messages[previous_history_len..])
                .await?;
        }
        *history = workflow_output.messages;
        Ok(workflow_output.output)
    }

    async fn ensure_session_started(&self) -> Result<()> {
        let mut started = self.session.session_started.lock().await;
        if *started {
            return Ok(());
        }

        self.services
            .events
            .emit(
                EventContext::new(self.session.session_id, self.session.thread_id, None),
                Event::SessionStarted {
                    session_id: self.session.session_id,
                    cwd: self.services.cwd.clone(),
                },
            )
            .await?;
        *started = true;
        Ok(())
    }

    pub async fn render(&self, output: &AgentOutput) -> Result<String> {
        let json = agent_contracts::abi_stable::std_types::RString::from(
            serde_json::to_string(output)?,
        );
        match self.services.registry.renderer.render_json(json) {
            agent_contracts::abi_stable::std_types::RResult::ROk(text) => {
                Ok(text.into_string())
            }
            agent_contracts::abi_stable::std_types::RResult::RErr(err) => {
                Err(anyhow::anyhow!("renderer error: {}", err.message))
            }
        }
    }

    pub async fn clear_history(&self) -> Result<()> {
        let _run_guard = self.session.run_lock.lock().await;
        self.session.history.lock().await.clear();
        if let Some(session_store) = &self.session.session_store {
            session_store.clear().await?;
        }
        Ok(())
    }

    pub async fn history_len(&self) -> usize {
        self.session.history.lock().await.len()
    }

    pub fn session_dir(&self) -> Option<&std::path::Path> {
        self.session
            .session_store
            .as_ref()
            .map(|store| store.session_dir())
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
    session_id: Option<SessionId>,
    thread_id: Option<ThreadId>,
    session_dir: Option<PathBuf>,
    resume_history: bool,
}

impl AgentRuntimeBuilder {
    pub fn new(config: AppConfig, cwd: PathBuf) -> Self {
        Self {
            config,
            cwd,
            config_path: None,
            event_sink: None,
            approval: None,
            session_id: None,
            thread_id: None,
            session_dir: None,
            resume_history: false,
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

    pub fn with_session_ids(mut self, session_id: SessionId, thread_id: ThreadId) -> Self {
        self.session_id = Some(session_id);
        self.thread_id = Some(thread_id);
        self
    }

    pub fn resume_from_session_dir(
        mut self,
        session_dir: impl Into<PathBuf>,
        session_id: SessionId,
        thread_id: ThreadId,
    ) -> Self {
        self.session_dir = Some(session_dir.into());
        self.session_id = Some(session_id);
        self.thread_id = Some(thread_id);
        self.resume_history = true;
        self
    }

    pub fn build(self) -> Result<AgentRuntime> {
        let Self {
            config,
            cwd,
            config_path,
            event_sink,
            approval,
            session_id,
            thread_id,
            session_dir,
            resume_history,
        } = self;

        let permission_mode = config.permissions.mode;
        let registry = BuiltinRegistry::from_config(&config, cwd.clone())?;
        let event_sink: Arc<dyn EventSink> = event_sink
            .unwrap_or_else(|| Arc::new(JsonlEventStore::new(cwd.join(&config.event_log.path))));
        let events = Arc::new(EventEmitter::new(event_sink));
        let approval: Arc<dyn ApprovalTransport> = Arc::new(CachedApprovalTransport::new(
            approval.unwrap_or_else(|| Arc::new(HeadlessApprovalTransport)),
        ));
        let session_id = session_id.unwrap_or_else(new_session_id);
        let thread_id = thread_id.unwrap_or_else(new_thread_id);
        let session_store = if let Some(session_dir) = session_dir {
            Some(SessionStore::from_session_dir(session_dir))
        } else {
            config_path
                .as_deref()
                .map(config_store_root)
                .map(|config_dir| SessionStore::new(&config_dir, &cwd, session_id))
        };
        let history = if resume_history {
            session_store
                .as_ref()
                .map(SessionStore::load_messages)
                .transpose()?
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let session_started = resume_history && !history.is_empty();

        Ok(AgentRuntime {
            services: RuntimeServices {
                cwd,
                registry,
                events,
                approval,
                permission_mode,
            },
            session: SessionState::new(
                session_id,
                thread_id,
                session_store,
                history,
                session_started,
            ),
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
            Ok(WorkflowOutput::new(
                AgentOutput::text("bad workflow"),
                Vec::new(),
            ))
        }
    }

    #[tokio::test]
    async fn run_errors_when_workflow_drops_existing_history() {
        let cwd = tempfile::tempdir().expect("temp dir");
        let mut runtime = AgentRuntime::builder(AppConfig::default(), cwd.path().to_path_buf())
            .build()
            .expect("runtime");

        runtime.services.registry.workflow = Arc::new(ShortHistoryWorkflow);
        runtime
            .session
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
