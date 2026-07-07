use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use crate::contracts::{RequestOrigin, UserInputRequest, UserInputResponse, UserInputTransport};

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

/// Обёртка, штампующая attribution на user-input запросы. Tools строят
/// `UserInputRequest` сами и не знают thread/turn; orchestrator оборачивает
/// транспорт этой обёрткой при сборке `ToolContext`, чтобы каждый запрос нес
/// `RequestOrigin` исполняющего контекста. Уже проставленный origin не
/// перезаписывается.
pub struct AttributedUserInputTransport {
    inner: Arc<dyn UserInputTransport>,
    origin: RequestOrigin,
}

impl AttributedUserInputTransport {
    pub fn new(inner: Arc<dyn UserInputTransport>, origin: RequestOrigin) -> Self {
        Self { inner, origin }
    }
}

#[async_trait]
impl UserInputTransport for AttributedUserInputTransport {
    fn can_request_user_input(&self) -> bool {
        self.inner.can_request_user_input()
    }

    async fn request_user_input(&self, request: UserInputRequest) -> Result<UserInputResponse> {
        let request = if request.origin.is_none() {
            request.with_origin(self.origin.clone())
        } else {
            request
        };
        self.inner.request_user_input(request).await
    }
}
