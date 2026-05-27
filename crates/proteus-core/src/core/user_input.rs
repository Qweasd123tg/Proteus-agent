use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use crate::contracts::{UserInputRequest, UserInputResponse, UserInputTransport};

pub struct PendingUserInput {
    pub request: UserInputRequest,
    pub responder: oneshot::Sender<UserInputResponse>,
}

#[derive(Clone)]
pub struct ChannelUserInputTransport {
    tx: mpsc::Sender<PendingUserInput>,
}

impl ChannelUserInputTransport {
    pub fn new(capacity: usize) -> (Self, mpsc::Receiver<PendingUserInput>) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (Self { tx }, rx)
    }
}

#[async_trait]
impl UserInputTransport for ChannelUserInputTransport {
    fn can_request_user_input(&self) -> bool {
        true
    }

    async fn request_user_input(&self, request: UserInputRequest) -> Result<UserInputResponse> {
        let (responder, rx) = oneshot::channel();
        self.tx
            .send(PendingUserInput { request, responder })
            .await
            .map_err(|_| anyhow!("user input transport: front-end is gone"))?;
        rx.await
            .map_err(|_| anyhow!("user input transport: response channel dropped"))
    }
}

#[derive(Debug, Default)]
pub struct HeadlessUserInputTransport;

#[async_trait]
impl UserInputTransport for HeadlessUserInputTransport {
    fn can_request_user_input(&self) -> bool {
        false
    }

    async fn request_user_input(&self, _request: UserInputRequest) -> Result<UserInputResponse> {
        Err(anyhow!("user input transport is not interactive"))
    }
}
