use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, anyhow};
use tokio::sync::{Mutex, RwLock};

use crate::{
    contracts::{ApprovalTransport, EventEmitter, EventSink, UserInputTransport},
    core::{
        AppConfig, CachedApprovalTransport, HeadlessApprovalTransport, HeadlessUserInputTransport,
        JsonlEventStore, ModuleCatalog, PreparedAssembly, SessionConfigSnapshot, SessionStore,
        write_config_snapshot,
    },
    domain::{SessionId, ThreadId, new_session_id, new_thread_id},
};

use super::{
    AgentRuntime, ModuleEpoch, RuntimeExecutionState, RuntimeServices, RuntimeSnapshot,
    SessionState, config_store_root, event_log_path,
};

/// Builder for `AgentRuntime`. Every slot has a sensible default
/// (headless approval, jsonl event log derived from the config, no session
/// persistence) so callers only override what they actually want to change.
pub struct AgentRuntimeBuilder {
    config: AppConfig,
    cwd: PathBuf,
    module_catalog: Option<ModuleCatalog>,
    config_path: Option<PathBuf>,
    event_sink: Option<Arc<dyn EventSink>>,
    approval: Option<Arc<dyn ApprovalTransport>>,
    user_input: Option<Arc<dyn UserInputTransport>>,
    session_id: Option<SessionId>,
    thread_id: Option<ThreadId>,
    resumed_session: Option<SessionStore>,
}

impl AgentRuntimeBuilder {
    pub fn new(config: AppConfig, cwd: PathBuf) -> Self {
        Self {
            config,
            cwd,
            module_catalog: None,
            config_path: None,
            event_sink: None,
            approval: None,
            user_input: None,
            session_id: None,
            thread_id: None,
            resumed_session: None,
        }
    }

    pub fn with_config_path(mut self, path: Option<&Path>) -> Self {
        self.config_path = path.map(Path::to_path_buf);
        self
    }

    pub fn with_module_catalog(mut self, catalog: ModuleCatalog) -> Self {
        self.module_catalog = Some(catalog);
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

    pub fn with_user_input(mut self, user_input: Arc<dyn UserInputTransport>) -> Self {
        self.user_input = Some(user_input);
        self
    }

    pub fn with_session_ids(mut self, session_id: SessionId, thread_id: ThreadId) -> Self {
        self.session_id = Some(session_id);
        self.thread_id = Some(thread_id);
        self
    }

    pub fn resume_from_session_dir(
        self,
        session_dir: impl Into<PathBuf>,
        thread_id: ThreadId,
    ) -> Result<Self> {
        let session_store = SessionStore::open(session_dir.into())?;
        Ok(self.resume_from_session_store(session_store, thread_id))
    }

    pub(crate) fn resume_from_session_store(
        mut self,
        session_store: SessionStore,
        thread_id: ThreadId,
    ) -> Self {
        self.thread_id = Some(thread_id);
        self.resumed_session = Some(session_store);
        self
    }

    /// Builds a runtime without running synchronous module factories on a
    /// Tokio worker. Process-backed modules may spawn and handshake during
    /// registry construction, so async entrypoints must use this method.
    pub async fn build_async(self) -> Result<AgentRuntime> {
        tokio::task::spawn_blocking(move || self.build())
            .await
            .map_err(|error| anyhow!("runtime builder blocking task failed: {error}"))?
    }

    pub fn build(self) -> Result<AgentRuntime> {
        let Self {
            config,
            cwd,
            module_catalog,
            config_path,
            event_sink,
            approval,
            user_input,
            session_id,
            thread_id,
            resumed_session,
        } = self;

        let cwd = match resumed_session.as_ref() {
            Some(store) => store.workspace_path()?,
            None => cwd,
        };
        let assembly = if let Some(catalog) = module_catalog {
            PreparedAssembly::from_catalog(config, cwd.clone(), config_path.as_deref(), catalog)?
        } else {
            PreparedAssembly::from_config(config, cwd.clone(), config_path.as_deref())?
        };
        let config = assembly.plan().config();
        let registry = assembly.registry();
        let permission_mode = assembly.plan().permission_mode;
        let event_sink: Arc<dyn EventSink> = event_sink.unwrap_or_else(|| {
            let event_log_path =
                event_log_path(&config.event_log.path, config_path.as_deref(), &cwd);
            let raw: Arc<dyn EventSink> = Arc::new(JsonlEventStore::new(event_log_path));
            if config.event_log.persist_deltas {
                raw
            } else {
                // Фильтруем дельты из durable JSONL. Кастомный `event_sink`
                // (выставленный через builder) не трогаем — пользователь
                // может сам управлять что записывать, например в
                // AppServer'е где нужно и broadcast без фильтра.
                Arc::new(crate::contracts::FilteredEventSink::new(raw, |event| {
                    !crate::contracts::is_streaming_delta(event)
                }))
            }
        });
        let events = Arc::new(EventEmitter::new(event_sink));
        let approval: Arc<dyn ApprovalTransport> = Arc::new(CachedApprovalTransport::new(
            approval.unwrap_or_else(|| Arc::new(HeadlessApprovalTransport)),
        ));
        let user_input: Arc<dyn UserInputTransport> =
            user_input.unwrap_or_else(|| Arc::new(HeadlessUserInputTransport));
        let session_id = resumed_session
            .as_ref()
            .map(SessionStore::session_id)
            .or(session_id)
            .unwrap_or_else(new_session_id);
        let thread_id = thread_id.unwrap_or_else(new_thread_id);
        let resume_history = resumed_session.is_some();
        let session_store = if let Some(session_store) = resumed_session {
            Some(session_store)
        } else {
            config_path
                .as_deref()
                .map(config_store_root)
                .map(|config_dir| SessionStore::new(&config_dir, &cwd, session_id))
                .transpose()?
        };
        let config_snapshot = session_store.as_ref().map(|_| {
            SessionConfigSnapshot::from_runtime_config(&config, &registry, permission_mode)
        });
        if resume_history
            && let (Some(session_store), Some(snapshot)) = (&session_store, &config_snapshot)
            && session_store.session_dir().exists()
            && let Err(error) = write_config_snapshot(session_store.session_dir(), snapshot)
        {
            eprintln!("warning: failed to persist session config snapshot: {error:#}");
        }
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
        let model_ref = registry.model_config.model_ref();
        let reasoning = registry.model_config.reasoning.clone();
        let default_reasoning = reasoning.clone();

        Ok(AgentRuntime {
            services: RuntimeServices {
                cwd,
                execution_state: RwLock::new(RuntimeExecutionState {
                    runtime: RuntimeSnapshot::new(
                        ModuleEpoch::initial(),
                        assembly,
                        config_snapshot,
                    ),
                    permission_mode,
                    model_ref,
                    reasoning,
                }),
                reload_lock: Mutex::new(()),
                events,
                approval,
                user_input,
                default_reasoning,
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
