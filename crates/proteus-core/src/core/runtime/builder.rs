use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use tokio::sync::{Mutex, RwLock};

use crate::{
    contracts::{ApprovalTransport, EventEmitter, EventSink, UserInputTransport},
    core::{
        AppConfig, BuiltinModuleCatalog, BuiltinRegistry, CachedApprovalTransport,
        HeadlessApprovalTransport, HeadlessUserInputTransport, JsonlEventStore, SessionStore,
    },
    domain::{SessionId, ThreadId, new_session_id, new_thread_id},
};

use super::{
    AgentRuntime, ModuleEpoch, RuntimeServices, RuntimeSnapshot, SessionState, config_store_root,
    event_log_path,
};

/// Builder for `AgentRuntime`. Every slot has a sensible default
/// (headless approval, jsonl event log derived from the config, no session
/// persistence) so callers only override what they actually want to change.
pub struct AgentRuntimeBuilder {
    config: AppConfig,
    cwd: PathBuf,
    module_catalog: Option<BuiltinModuleCatalog>,
    config_path: Option<PathBuf>,
    event_sink: Option<Arc<dyn EventSink>>,
    approval: Option<Arc<dyn ApprovalTransport>>,
    user_input: Option<Arc<dyn UserInputTransport>>,
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
            module_catalog: None,
            config_path: None,
            event_sink: None,
            approval: None,
            user_input: None,
            session_id: None,
            thread_id: None,
            session_dir: None,
            resume_history: false,
        }
    }

    pub fn with_config_path(mut self, path: Option<&Path>) -> Self {
        self.config_path = path.map(Path::to_path_buf);
        self
    }

    pub fn with_module_catalog(mut self, catalog: BuiltinModuleCatalog) -> Self {
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
        mut self,
        session_dir: impl Into<PathBuf>,
        session_id: SessionId,
        thread_id: ThreadId,
    ) -> Self {
        let session_dir = session_dir.into();
        if let Ok(Some(workspace_path)) =
            crate::core::session_workspace_from_session_dir(&session_dir)
        {
            self.cwd = workspace_path;
        }
        self.session_dir = Some(session_dir);
        self.session_id = Some(session_id);
        self.thread_id = Some(thread_id);
        self.resume_history = true;
        self
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
            session_dir,
            resume_history,
        } = self;

        let permission_mode = config.permissions.mode;
        let registry = if let Some(catalog) = module_catalog {
            BuiltinRegistry::from_catalog(&config, cwd.clone(), catalog)?
        } else {
            BuiltinRegistry::from_config(&config, cwd.clone())?
        };
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
        let model_ref = registry.model_config.model_ref();
        let reasoning = registry.model_config.reasoning.clone();
        let default_reasoning = reasoning.clone();

        Ok(AgentRuntime {
            services: RuntimeServices {
                cwd,
                snapshot: RwLock::new(RuntimeSnapshot::new(ModuleEpoch::initial(), registry)),
                reload_lock: Mutex::new(()),
                events,
                approval,
                user_input,
                permission_mode: RwLock::new(permission_mode),
                model_ref: RwLock::new(model_ref),
                reasoning: RwLock::new(reasoning),
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
