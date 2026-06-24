use std::{collections::HashMap, sync::Arc};

use tokio::sync::{Mutex, broadcast};

use crate::contracts::CancellationToken;

use super::{AppServerHandle, security::HttpSecurity};

#[derive(Clone)]
pub(super) struct HttpAppState {
    pub(super) server: Arc<Mutex<AppServerHandle>>,
    pub(super) running_turns: Arc<Mutex<HashMap<String, CancellationToken>>>,
    pub(super) shutdown: broadcast::Sender<()>,
    pub(super) security: HttpSecurity,
}

impl HttpAppState {
    pub(super) fn new(
        server: AppServerHandle,
        shutdown: broadcast::Sender<()>,
        security: HttpSecurity,
    ) -> Self {
        Self {
            server: Arc::new(Mutex::new(server)),
            running_turns: Arc::new(Mutex::new(HashMap::new())),
            shutdown,
            security,
        }
    }

    pub(super) async fn current_server(&self) -> AppServerHandle {
        self.server.lock().await.clone()
    }
}
