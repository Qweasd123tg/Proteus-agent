use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use crate::contracts::{ApprovalRequest, ApprovalResponse, ApprovalTransport};

/// Pairs an approval request with a one-shot responder. The transport sends
/// one of these into a channel; a front-end or remote UI dequeues it,
/// shows the prompt, and answers via the responder.
pub struct PendingApproval {
    pub request: ApprovalRequest,
    pub responder: oneshot::Sender<ApprovalResponse>,
}

/// Generic channel-backed approval transport. Decoupled from any specific UI —
/// a REPL, RPC server, external UI or test harness can drive it through the
/// receiver end of the channel.
#[derive(Clone)]
pub struct ChannelApprovalTransport {
    tx: mpsc::Sender<PendingApproval>,
}

impl ChannelApprovalTransport {
    pub fn new(capacity: usize) -> (Self, mpsc::Receiver<PendingApproval>) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (Self { tx }, rx)
    }
}

#[async_trait]
impl ApprovalTransport for ChannelApprovalTransport {
    fn can_request_approval(&self) -> bool {
        true
    }

    async fn request_approval(&self, request: ApprovalRequest) -> Result<ApprovalResponse> {
        let (responder, rx) = oneshot::channel();
        self.tx
            .send(PendingApproval { request, responder })
            .await
            .map_err(|_| anyhow!("approval transport: front-end is gone"))?;
        rx.await
            .map_err(|_| anyhow!("approval transport: response channel dropped"))
    }
}
