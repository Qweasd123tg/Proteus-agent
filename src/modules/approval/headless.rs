use anyhow::Result;
use async_trait::async_trait;

use crate::contracts::{ApprovalCacheScope, ApprovalRequest, ApprovalResponse, ApprovalTransport};

#[derive(Debug, Default)]
pub struct HeadlessApprovalTransport;

#[async_trait]
impl ApprovalTransport for HeadlessApprovalTransport {
    fn can_request_approval(&self) -> bool {
        false
    }

    async fn request_approval(&self, request: ApprovalRequest) -> Result<ApprovalResponse> {
        Ok(ApprovalResponse {
            approved: false,
            note: Some(format!(
                "approval transport is not interactive: {}",
                request.reason
            )),
            cache: ApprovalCacheScope::None,
        })
    }
}
