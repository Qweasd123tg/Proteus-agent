use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
};

use tokio::sync::{Mutex, broadcast};

use crate::core::AppConfig;

use super::{AppServerEvent, AppServerHandle, AppSessionActivity, security::HttpSecurity};

#[derive(Clone)]
pub(super) struct HttpLaunchContext {
    pub(super) config: AppConfig,
    pub(super) cwd: PathBuf,
    pub(super) config_path: Option<PathBuf>,
    pub(super) initial_session_dir: Option<PathBuf>,
}

#[derive(Clone)]
pub(super) struct HttpAppState {
    pub(super) launch: Arc<HttpLaunchContext>,
    // Serialize registry lifecycle changes, so concurrent resume cannot create
    // two runtimes writing the same session journal.
    pub(super) session_lifecycle: Arc<Mutex<()>>,
    pub(super) session_servers: Arc<Mutex<HashMap<PathBuf, AppServerHandle>>>,
    activity_events: broadcast::Sender<AppServerEvent>,
    watched_sessions: Arc<StdMutex<HashSet<PathBuf>>>,
    pub(super) shutdown: broadcast::Sender<()>,
    pub(super) security: HttpSecurity,
}

impl HttpAppState {
    pub(super) async fn new(
        server: AppServerHandle,
        shutdown: broadcast::Sender<()>,
        security: HttpSecurity,
    ) -> Self {
        let initial_server = server.clone();
        let launch = HttpLaunchContext {
            config: server.config.read().await.clone(),
            cwd: server.cwd.clone(),
            config_path: server.config_path.clone(),
            initial_session_dir: server.session_dir_path(),
        };
        let mut session_servers = HashMap::new();
        if let Some(session_dir) = server.session_dir_path() {
            session_servers.insert(session_key(session_dir), server.clone());
        }
        let (activity_events, _) = broadcast::channel(1024);
        let state = Self {
            launch: Arc::new(launch),
            session_lifecycle: Arc::new(Mutex::new(())),
            session_servers: Arc::new(Mutex::new(session_servers)),
            activity_events,
            watched_sessions: Arc::new(StdMutex::new(HashSet::new())),
            shutdown,
            security,
        };
        state.watch_server(initial_server);
        state
    }

    pub(super) fn subscribe_activity(&self) -> broadcast::Receiver<AppServerEvent> {
        self.activity_events.subscribe()
    }

    pub(super) async fn remember_server(&self, server: AppServerHandle) {
        if let Some(session_dir) = server.session_dir_path() {
            let key = session_key(session_dir.clone());
            self.session_servers
                .lock()
                .await
                .insert(key.clone(), server.clone());
            self.watch_server(server);
            self.emit_session_activity_for_dir(&key).await;
        }
    }

    pub(super) async fn remove_session_server(
        &self,
        session_dir: &Path,
    ) -> Option<AppServerHandle> {
        let key = session_key(session_dir.to_path_buf());
        self.watched_sessions
            .lock()
            .expect("session watcher lock")
            .remove(&key);
        self.session_servers.lock().await.remove(&key)
    }

    pub(super) async fn server_for_session_dir(
        &self,
        session_dir: &Path,
    ) -> Option<AppServerHandle> {
        let key = session_key(session_dir.to_path_buf());
        self.session_servers.lock().await.get(&key).cloned()
    }

    pub(super) async fn all_servers(&self) -> Vec<AppServerHandle> {
        self.session_servers
            .lock()
            .await
            .values()
            .cloned()
            .collect()
    }

    pub(super) async fn activity_for_server(&self, server: &AppServerHandle) -> AppSessionActivity {
        server
            .session_activity(server.running_run_ids().await)
            .await
    }

    pub(super) async fn activity_by_session_dir(&self) -> HashMap<PathBuf, AppSessionActivity> {
        let mut activity = HashMap::new();
        for server in self.all_servers().await {
            if let Some(session_dir) = server.session_dir_path() {
                activity.insert(
                    session_key(session_dir),
                    self.activity_for_server(&server).await,
                );
            }
        }
        activity
    }

    pub(super) async fn emit_session_activity_for_dir(&self, session_dir: &Path) {
        let Some(server) = self.server_for_session_dir(session_dir).await else {
            return;
        };
        let session_dir = session_key(session_dir.to_path_buf());
        let activity = self.activity_for_server(&server).await;
        let _ = self
            .activity_events
            .send(AppServerEvent::SessionActivityUpdated {
                session_dir: session_dir.to_path_buf(),
                activity,
            });
    }

    pub(super) async fn emit_session_activity_for_server(&self, server: &AppServerHandle) {
        if let Some(session_dir) = server.session_dir_path() {
            self.emit_session_activity_for_dir(&session_dir).await;
        }
    }

    fn watch_server(&self, server: AppServerHandle) {
        let Some(session_dir) = server.session_dir_path() else {
            return;
        };
        let session_dir = session_key(session_dir);
        {
            let mut watched = self
                .watched_sessions
                .lock()
                .expect("watched session lock poisoned");
            if !watched.insert(session_dir.clone()) {
                return;
            }
        }

        let state = self.clone();
        let mut events = server.subscribe();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let should_stop = matches!(event, AppServerEvent::Shutdown);
                        if app_event_affects_session_activity(&event) {
                            state.emit_session_activity_for_dir(&session_dir).await;
                        }
                        if should_stop {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        state.emit_session_activity_for_dir(&session_dir).await;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
}

fn app_event_affects_session_activity(event: &AppServerEvent) -> bool {
    matches!(
        event,
        AppServerEvent::UserMessageSubmitted { .. }
            | AppServerEvent::ExecutionUpdated { .. }
            | AppServerEvent::TurnOutput { .. }
            | AppServerEvent::ApprovalRequested { .. }
            | AppServerEvent::ApprovalResolved { .. }
            | AppServerEvent::UserInputRequested { .. }
            | AppServerEvent::UserInputResolved { .. }
            | AppServerEvent::Error { .. }
            | AppServerEvent::Shutdown
    )
}

pub(super) fn session_key(session_dir: PathBuf) -> PathBuf {
    crate::core::canonicalize_session_dir_path(session_dir.clone()).unwrap_or(session_dir)
}
