use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{
    contracts::{ExecutionRecorder, ModelCallOrigin},
    domain::ExchangeId,
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, ModelFailure, ModelStreamEvent,
    },
};

use super::ModelContextState;

/// Preserve in-memory continuation even when the runtime has no session store.
/// Persisted facts are acknowledged first; a new turn reloads committed journal
/// facts so a lost append acknowledgement cannot split warm and cold behavior.
pub(crate) struct ContextExecutionRecorder {
    pub(crate) inner: Arc<dyn ExecutionRecorder>,
    pub(crate) context: Arc<Mutex<ModelContextState>>,
}

#[async_trait]
impl ExecutionRecorder for ContextExecutionRecorder {
    async fn model_request_recorded(
        &self,
        exchange_id: ExchangeId,
        origin: ModelCallOrigin,
        request: &CanonicalModelRequest,
    ) -> Result<()> {
        self.inner
            .model_request_recorded(exchange_id, origin, request)
            .await?;
        self.context
            .lock()
            .await
            .request(exchange_id, origin, request);
        Ok(())
    }

    async fn model_stream_event_recorded(
        &self,
        exchange_id: ExchangeId,
        event: &ModelStreamEvent,
    ) -> Result<()> {
        self.inner
            .model_stream_event_recorded(exchange_id, event)
            .await
    }

    async fn model_response_recorded(
        &self,
        exchange_id: ExchangeId,
        response: &CanonicalModelResponse,
    ) -> Result<()> {
        self.inner
            .model_response_recorded(exchange_id, response)
            .await?;
        self.context.lock().await.response(exchange_id, response);
        Ok(())
    }

    async fn model_error_recorded(
        &self,
        exchange_id: ExchangeId,
        failure: &ModelFailure,
    ) -> Result<()> {
        self.inner
            .model_error_recorded(exchange_id, failure)
            .await?;
        self.context.lock().await.failure(exchange_id, failure);
        Ok(())
    }
}
