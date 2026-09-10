//! Session-scoped reads and human responses shared by app-server transports.

use super::{
    AppPendingRequests, AppServerEvent, AppServerHandle, AppSessionActivity, approvals,
    user_inputs::{self, resolve_pending_user_inputs_empty},
};
use crate::contracts::{ApprovalCacheScope, ApprovalResponse, UserInputResponse};
use anyhow::Result;

impl AppServerHandle {
    pub async fn pending_requests(&self) -> AppPendingRequests {
        self.events.pending_snapshot()
    }

    pub async fn has_pending_approval(&self, approval_id: &str) -> bool {
        self.pending_approvals
            .lock()
            .await
            .contains_key(approval_id)
    }

    pub async fn has_pending_user_input(&self, request_id: &str) -> bool {
        self.pending_user_inputs
            .lock()
            .await
            .contains_key(request_id)
    }

    pub async fn session_activity(&self, running_run_ids: Vec<String>) -> AppSessionActivity {
        let pending_approvals = self.pending_approvals.lock().await.len();
        let pending_user_inputs = self.pending_user_inputs.lock().await.len();
        AppSessionActivity::from_running_run_ids(
            running_run_ids,
            pending_approvals,
            pending_user_inputs,
        )
    }

    pub async fn respond_approval(
        &self,
        approval_id: &str,
        approved: bool,
        note: Option<String>,
        cache: ApprovalCacheScope,
    ) -> Result<()> {
        approvals::resolve_pending_approval(
            &self.pending_approvals,
            &self.events,
            approval_id,
            ApprovalResponse::new(approved, note, cache),
        )
        .await
    }

    pub async fn respond_user_input(
        &self,
        request_id: &str,
        response: UserInputResponse,
    ) -> Result<()> {
        user_inputs::resolve_pending_user_input(
            &self.pending_user_inputs,
            &self.events,
            request_id,
            response,
        )
        .await
    }

    pub async fn shutdown(&self) {
        self.close_runs().await;
        approvals::deny_pending_approvals(
            self.pending_approvals.clone(),
            &self.events,
            "app-server shutting down".to_owned(),
        )
        .await;
        resolve_pending_user_inputs_empty(self.pending_user_inputs.clone(), &self.events).await;
        let _ = self.events.send(AppServerEvent::Shutdown);
    }
}
