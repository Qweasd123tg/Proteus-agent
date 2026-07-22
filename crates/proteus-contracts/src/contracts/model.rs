use std::{borrow::Cow, pin::Pin};

use anyhow::Result;
use async_trait::async_trait;
use futures_util::StreamExt;

use crate::{
    domain::{ModelRef, ToolSpec},
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

    /// Configured provider-hosted tool instances for this model. The default
    /// keeps providers without hosted execution unchanged.
    fn provider_hosted_tools(&self, _model: &ModelRef) -> Vec<ToolSpec> {
        Vec::new()
    }

    /// Returns provider events normalized to the canonical stream contract.
    ///
    /// A successful stream must contain one terminal `Response` whose
    /// canonical message and tool calls are complete. Provider adapters must
    /// repair provider-specific partial/empty terminal payloads before they
    /// emit that response; consumers do not reconstruct it from deltas.
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
