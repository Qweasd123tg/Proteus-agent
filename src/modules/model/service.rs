use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{ModelAdapter, ModelClient, ModelEventStream},
    domain::ModelRef,
    model_standard::{CanonicalModelRequest, ModelCapabilities, RequestShaper},
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
    fn id(&self) -> std::borrow::Cow<'static, str> {
        self.adapter.id()
    }

    fn capabilities(&self, model: &ModelRef) -> ModelCapabilities {
        self.adapter.capabilities(model)
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let capabilities = self.adapter.capabilities(&request.model);
        let request = self.shaper.shape(request, &capabilities)?;
        self.adapter.stream(request).await
    }
}
