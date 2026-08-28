use anyhow::Result;
use async_trait::async_trait;

use crate::{
    domain::ExchangeId,
    model_standard::{CanonicalModelRequest, CanonicalModelResponse},
};

/// Execution-bound sink for generic model lifecycle facts.
///
/// The binding selects the execution owner before this surface reaches a
/// caller, so individual calls deliberately carry no session, thread or turn
/// identity. Implementations may persist the facts or keep them in memory.
#[async_trait]
pub trait ExecutionRecorder: Send + Sync {
    async fn model_request_recorded(
        &self,
        exchange_id: ExchangeId,
        request: &CanonicalModelRequest,
    ) -> Result<()>;

    async fn model_response_recorded(
        &self,
        exchange_id: ExchangeId,
        response: &CanonicalModelResponse,
    ) -> Result<()>;

    async fn model_error_recorded(&self, exchange_id: ExchangeId, message: &str) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct NoopExecutionRecorder;

#[async_trait]
impl ExecutionRecorder for NoopExecutionRecorder {
    async fn model_request_recorded(
        &self,
        _exchange_id: ExchangeId,
        _request: &CanonicalModelRequest,
    ) -> Result<()> {
        Ok(())
    }

    async fn model_response_recorded(
        &self,
        _exchange_id: ExchangeId,
        _response: &CanonicalModelResponse,
    ) -> Result<()> {
        Ok(())
    }

    async fn model_error_recorded(&self, _exchange_id: ExchangeId, _message: &str) -> Result<()> {
        Ok(())
    }
}
