use std::{collections::HashMap, path::PathBuf, sync::Arc};

use a2a::{A2AError, Message, Part, Role, Task, TaskState};
use tokio::sync::{Mutex, OnceCell, watch};

use super::A2aServerConfig;
use crate::{app_server::AppServerHandle, contracts::CancellationToken, core::AppConfig};

#[derive(Clone)]
pub(super) struct Service(Arc<Inner>);

pub(super) struct Inner {
    pub config: AppConfig,
    pub cwd: PathBuf,
    pub config_path: Option<PathBuf>,
    pub limits: A2aServerConfig,
    pub registry: Mutex<Registry>,
}

impl std::ops::Deref for Service {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

#[derive(Default)]
pub(super) struct Registry {
    pub contexts: HashMap<String, Arc<Context>>,
    pub tasks: HashMap<String, Arc<Run>>,
    pub closed: bool,
}

#[derive(Default)]
pub(super) struct Context {
    pub server: OnceCell<AppServerHandle>,
}

pub(super) struct Run {
    pub context: Arc<Context>,
    pub cancellation: CancellationToken,
    pub updates: watch::Sender<Task>,
}

impl Service {
    pub fn new(
        config: AppConfig,
        cwd: PathBuf,
        config_path: Option<PathBuf>,
        limits: A2aServerConfig,
    ) -> Self {
        Self(Arc::new(Inner {
            config,
            cwd,
            config_path,
            limits,
            registry: Mutex::new(Registry::default()),
        }))
    }

    pub async fn lookup(&self, id: &str) -> Result<Arc<Run>, A2AError> {
        self.registry
            .lock()
            .await
            .tasks
            .get(id)
            .cloned()
            .ok_or_else(|| A2AError::task_not_found(id))
    }

    // Admission, cancellation and publication use one lock. Terminal state is
    // published only after the app-server has settled its canonical journal.
    pub async fn publish(&self, run: &Run, state: TaskState, parts: Vec<Part>) {
        let _registry = self.registry.lock().await;
        self.publish_locked(run, state, parts).await;
    }

    pub async fn publish_pending(&self, run: &Run) {
        let _registry = self.registry.lock().await;
        let Some(server) = run.context.server.get() else {
            return;
        };
        // Read after taking the admission lock. A response can resolve the
        // request while its earlier pending event waits for this lock.
        let pending = server.pending_requests().await;
        if let Some(parts) = super::interaction::pending_parts(&pending) {
            self.publish_locked(run, TaskState::InputRequired, parts)
                .await;
        } else {
            let waiting = run.updates.borrow().status.state == TaskState::InputRequired;
            if waiting {
                self.publish_locked(run, TaskState::Working, vec![]).await;
            }
        }
    }

    async fn publish_locked(&self, run: &Run, state: TaskState, parts: Vec<Part>) {
        let mut task = run.updates.borrow().clone();
        if task.status.state.is_terminal() {
            return;
        }
        task.status.state = state.clone();
        let has_content = !parts.is_empty();
        let mut message = Message::new(Role::Agent, parts);
        message.task_id = Some(task.id.clone());
        message.context_id = Some(task.context_id.clone());
        if state == TaskState::InputRequired {
            message.extensions = Some(vec![super::interaction::INTERACTION_EXTENSION.into()]);
        }
        task.status.message = has_content.then_some(message.clone());
        if state.is_terminal() && has_content {
            task.history.get_or_insert_default().push(message);
        }
        run.updates.send_replace(task);
    }

    pub async fn shutdown(&self) {
        let (runs, contexts) = {
            let mut registry = self.registry.lock().await;
            registry.closed = true;
            for run in registry.tasks.values() {
                run.cancellation.cancel();
            }
            (
                registry.tasks.values().cloned().collect::<Vec<_>>(),
                registry.contexts.values().cloned().collect::<Vec<_>>(),
            )
        };
        for run in runs {
            let _ = settled(run.updates.subscribe()).await;
        }
        for context in contexts {
            if let Some(server) = context.server.get() {
                server.shutdown().await;
            }
        }
    }
}

pub(super) async fn settled(mut updates: watch::Receiver<Task>) -> Result<Task, A2AError> {
    loop {
        let task = updates.borrow_and_update().clone();
        if task.status.state.is_terminal() {
            return Ok(task);
        }
        updates
            .changed()
            .await
            .map_err(|_| A2AError::internal("task publisher closed before settlement"))?;
    }
}
