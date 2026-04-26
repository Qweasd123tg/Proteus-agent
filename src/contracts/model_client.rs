use anyhow::Result;
use async_trait::async_trait;

use crate::model_standard::{CanonicalModelRequest, CanonicalModelResponse};

#[async_trait]
pub trait ModelClient: Send + Sync {
    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse>;
}
