use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{ModelAdapter, ModelClient},
    model_standard::{CanonicalModelRequest, CanonicalModelResponse, RequestShaper},
};

pub struct ModelService {
    adapter: Arc<dyn ModelAdapter>,
    shaper: RequestShaper,
}

impl ModelService {
    pub fn new(adapter: Arc<dyn ModelAdapter>) -> Self {
        Self {
            adapter,
            shaper: RequestShaper,
        }
    }

    pub fn with_shaper(adapter: Arc<dyn ModelAdapter>, shaper: RequestShaper) -> Self {
        Self { adapter, shaper }
    }
}

#[async_trait]
impl ModelClient for ModelService {
    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        let capabilities = self.adapter.capabilities(&request.model);
        let request = self.shaper.shape(request, &capabilities)?;
        self.adapter.complete(request).await
    }
}
