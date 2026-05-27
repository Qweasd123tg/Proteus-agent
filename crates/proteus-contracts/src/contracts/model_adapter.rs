use std::{borrow::Cow, pin::Pin};

use anyhow::Result;
use async_trait::async_trait;
use futures_util::StreamExt;

use crate::{
    domain::ModelRef,
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, ModelCapabilities, ModelStreamEvent,
    },
};

pub type ModelEventStream =
    Pin<Box<dyn futures_core::Stream<Item = Result<ModelStreamEvent>> + Send>>;

#[async_trait]
pub trait Model: Send + Sync {
    fn id(&self) -> Cow<'static, str>;
    fn capabilities(&self, model: &ModelRef) -> ModelCapabilities;

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream>;

    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        let mut stream = self.stream(request).await?;
        while let Some(event) = stream.next().await {
            if let ModelStreamEvent::Response { response } = event? {
                return Ok(response);
            }
        }
        anyhow::bail!("model stream ended without a complete response")
    }
}

pub use Model as ModelAdapter;
