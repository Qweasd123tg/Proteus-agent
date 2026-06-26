use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
};

use tokio::sync::{Mutex, broadcast};

use crate::contracts::CancellationToken;

use super::{AppServerEvent, AppServerHandle, AppSessionActivity, security::HttpSecurity};

#[derive(Clone)]
pub(super) struct RunningTurn {
    pub(super) cancellation: CancellationToken,
    pub(super) session_dir: Option<PathBuf>,
}

impl RunningTurn {
    pub(super) fn new(cancellation: CancellationToken, session_dir: Option<PathBuf>) -> Self {
        Self {
            cancellation,
            session_dir,
        }
    }
}

#[derive(Clone)]
pub(super) struct HttpAppState {
    pub(super) server: Arc<Mutex<AppServerHandle>>,
    pub(super) session_servers: Arc<Mutex<HashMap<PathBuf, AppServerHandle>>>,
    pub(super) running_turns: Arc<Mutex<HashMap<String, RunningTurn>>>,
    activity_events: broadcast::Sender<AppServerEvent>,
    watched_sessions: Arc<StdMutex<HashSet<PathBuf>>>,
    pub(super) shutdown: broadcast::Sender<()>,
    pub(super) security: HttpSecurity,
}

impl HttpAppState {
    pub(super) fn new(
        server: AppServerHandle,
        shutdown: broadcast::Sender<()>,
        security: HttpSecurity,
    ) -> Self {
        let initial_server = server.clone();
        let mut session_servers = HashMap::new();
        if let Some(session_dir) = server.session_dir_path() {
            session_servers.insert(session_dir, server.clone());
        }
        let (activity_events, _) = broadcast::channel(1024);
        let state = Self {
            server: Arc::new(Mutex::new(server)),
            session_servers: Arc::new(Mutex::new(session_servers)),
            running_turns: Arc::new(Mutex::new(HashMap::new())),
            activity_events,
            watched_sessions: Arc::new(StdMutex::new(HashSet::new())),
            shutdown,
            security,
        };
        state.watch_server(initial_server);
        state
    }

    pub(super) async fn current_server(&self) -> AppServerHandle {
        self.server.lock().await.clone()
    }

    pub(super) fn subscribe_activity(&self) -> broadcast::Receiver<AppServerEvent> {
        self.activity_events.subscribe()
    }

    pub(super) async fn set_current_server(&self, server: AppServerHandle) {
        self.remember_server(server.clone()).await;
        *self.server.lock().await = server;
    }

    pub(super) async fn remember_server(&self, server: AppServerHandle) {
        if let Some(session_dir) = server.session_dir_path() {
            self.session_servers
                .lock()
                .await
                .insert(session_dir.clone(), server.clone());
            self.watch_server(server);
            self.emit_session_activity_for_dir(&session_dir).await;
        }
    }

    pub(super) async fn remove_session_server(
        &self,
        session_dir: &Path,
    ) -> Option<AppServerHandle> {
        self.session_servers.lock().await.remove(session_dir)
    }

    pub(super) async fn server_for_session_dir(
        &self,
        session_dir: &Path,
    ) -> Option<AppServerHandle> {
        self.session_servers.lock().await.get(session_dir).cloned()
    }

    pub(super) async fn all_servers(&self) -> Vec<AppServerHandle> {
        let current = self.current_server().await;
        let mut servers = vec![current.clone()];
        let current_dir = current.session_dir_path();
        servers.extend(
            self.session_servers
                .lock()
                .await
                .iter()
                .filter(|(session_dir, _)| Some(*session_dir) != current_dir.as_ref())
                .map(|(_, server)| server.clone()),
        );
        servers
    }

    pub(super) async fn server_for_pending_approval(
        &self,
        approval_id: &str,
    ) -> Option<AppServerHandle> {
        for server in self.all_servers().await {
            if server.has_pending_approval(approval_id).await {
                return Some(server);
            }
        }
        None
    }

    pub(super) async fn server_for_pending_user_input(
        &self,
        request_id: &str,
    ) -> Option<AppServerHandle> {
        for server in self.all_servers().await {
            if server.has_pending_user_input(request_id).await {
                return Some(server);
            }
        }
        None
    }

    pub(super) async fn running_turn_count_for(&self, session_dir: Option<&Path>) -> usize {
        self.running_turns
            .lock()
            .await
            .values()
            .filter(|turn| match (turn.session_dir.as_deref(), session_dir) {
                (Some(left), Some(right)) => left == right,
                (None, None) => true,
                _ => false,
            })
            .count()
    }

    pub(super) async fn activity_for_server(&self, server: &AppServerHandle) -> AppSessionActivity {
        let running_turns = self
            .running_turn_count_for(server.session_dir_path().as_deref())
            .await;
        server.session_activity(running_turns).await
    }

    pub(super) async fn activity_by_session_dir(&self) -> HashMap<PathBuf, AppSessionActivity> {
        let mut activity = HashMap::new();
        for server in self.all_servers().await {
            if let Some(session_dir) = server.session_dir_path() {
                activity.insert(session_dir, self.activity_for_server(&server).await);
            }
        }
        activity
    }

    pub(super) async fn emit_session_activity_for_dir(&self, session_dir: &Path) {
        let Some(server) = self.server_for_session_dir(session_dir).await else {
            return;
        };
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
        tokio::spawn(async move {
            let mut events = server.subscribe();
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
            | AppServerEvent::TurnOutput { .. }
            | AppServerEvent::ApprovalRequested { .. }
            | AppServerEvent::ApprovalResolved { .. }
            | AppServerEvent::UserInputRequested { .. }
            | AppServerEvent::UserInputResolved { .. }
            | AppServerEvent::Error { .. }
            | AppServerEvent::Shutdown
    )
}
