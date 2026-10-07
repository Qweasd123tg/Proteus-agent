use super::{
    approvals::PendingApprovalResponders, events::AppEventPublisher, runs::RunRegistry,
    user_inputs::PendingUserInputResponders,
};
use crate::{
    contracts::CancellationToken,
    core::{AgentRuntime, AppConfig},
};
use std::{ops::Deref, path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, RwLock};

#[derive(Clone)]
pub struct AppServerHandle {
    pub(super) inner: Arc<AppServerState>,
}

/// Shared session state; fields remain app-server-owned.
#[doc(hidden)]
pub struct AppServerState {
    pub(super) runtime: Arc<AgentRuntime>,
    pub(super) config: Arc<RwLock<AppConfig>>,
    pub(super) config_path: Option<PathBuf>,
    pub(super) cwd: PathBuf,
    pub(super) events: AppEventPublisher,
    pub(super) pending_approvals: PendingApprovalResponders,
    pub(super) pending_user_inputs: PendingUserInputResponders,
    pub(super) runs: Arc<Mutex<RunRegistry>>,
    pub(super) profile_stop: CancellationToken,
    pub(super) profile_error: Mutex<Option<String>>,
}

impl Deref for AppServerHandle {
    type Target = AppServerState;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
