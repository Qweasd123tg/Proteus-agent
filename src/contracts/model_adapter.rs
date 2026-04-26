use anyhow::Result;
use async_trait::async_trait;

use crate::{
    domain::ModelRef,
    model_standard::{CanonicalModelRequest, CanonicalModelResponse, ModelCapabilities},
};

#[async_trait]
pub trait ModelAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn capabilities(&self, model: &ModelRef) -> ModelCapabilities;
    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse>;
}
