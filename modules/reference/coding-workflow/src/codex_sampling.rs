//! Reconnect an established model stream using the accepted conversation.
//! HTTP request retries remain owned by the model adapter.

use std::time::Duration;

use proteus_contracts::{
    domain::Event,
    model_standard::{CanonicalModelRequest, CanonicalModelResponse, ModelFailureKind},
    process_module::{ProcessModuleError, WorkflowModuleHostMut},
};
use rand::Rng;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    host::{complete_model, emit_event, ensure_not_cancelled},
    scaffold::TurnScaffold,
};

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamRetryConfig {
    #[serde(default = "default_stream_max_retries")]
    stream_max_retries: u64,
}

fn default_stream_max_retries() -> u64 {
    5
}

impl StreamRetryConfig {
    pub(crate) fn from_config(config: &Value) -> Result<Self, ProcessModuleError> {
        let mut config: Self = serde_json::from_value(config.clone()).map_err(|error| {
            ProcessModuleError::new(format!("invalid codex_loop config: {error}"))
        })?;
        config.stream_max_retries = config.stream_max_retries.min(100);
        Ok(config)
    }
}

pub(crate) fn complete_sampling_request(
    host: &mut WorkflowModuleHostMut<'_>,
    turn: &mut TurnScaffold,
    request: &mut CanonicalModelRequest,
    config: StreamRetryConfig,
) -> Result<CanonicalModelResponse, ProcessModuleError> {
    // Budget belongs to this sampling request, not to the whole turn and not
    // to each completed item emitted by an unsuccessful stream.
    let mut retries = 0;
    loop {
        ensure_not_cancelled(host)?;
        emit_event(
            host,
            &Event::ModelRequestPrepared {
                model: request.model.clone(),
            },
        )?;
        let error = match complete_model(host, request, "codex_loop") {
            Ok(response) => return Ok(response),
            Err(error) => error,
        };
        // Only direct model output belongs here. A compactor failure's summary
        // messages must never enter the conversation through this path.
        if let Some(failure) = &error.model_failure {
            turn.model_messages
                .extend(failure.completed_messages.iter().cloned());
            turn.persistent_messages
                .extend(failure.completed_messages.iter().cloned());
        }
        let disconnected = error
            .model_failure
            .as_ref()
            .is_some_and(|failure| failure.kind == ModelFailureKind::StreamDisconnected);
        if !disconnected || retries >= config.stream_max_retries {
            return Err(error);
        }
        // Persist accepted progress before waiting: a canceled reconnect must
        // leave the same completed history available to a cold reader.
        turn.checkpoint(host, &[])?;
        request.messages.clone_from(&turn.model_messages);
        retries += 1;
        wait_for_retry(host, retry_delay(retries))?;
    }
}

fn retry_delay(attempt: u64) -> Duration {
    let exponent = attempt.saturating_sub(1).min(63);
    let base = 200u64.saturating_mul(1u64 << exponent);
    let jitter = rand::rng().random_range(0.9..1.1);
    Duration::from_millis((base as f64 * jitter) as u64)
}

fn wait_for_retry(
    host: &mut WorkflowModuleHostMut<'_>,
    delay: Duration,
) -> Result<(), ProcessModuleError> {
    let mut remaining = delay;
    let slice = Duration::from_millis(20);
    while !remaining.is_zero() {
        ensure_not_cancelled(host)?;
        let current = remaining.min(slice);
        std::thread::sleep(current);
        remaining = remaining.saturating_sub(current);
    }
    ensure_not_cancelled(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stream_retry_configuration_is_bounded_and_strict() {
        assert_eq!(
            StreamRetryConfig::from_config(&json!({}))
                .unwrap()
                .stream_max_retries,
            5
        );
        assert_eq!(
            StreamRetryConfig::from_config(&json!({"stream_max_retries": 999}))
                .unwrap()
                .stream_max_retries,
            100
        );
        for config in [
            json!({"stream_max_retries": -1}),
            json!({"stream_max_retries": "5"}),
            json!({"stream_max_retries": 1.5}),
            json!({"unknown": 1}),
        ] {
            assert!(StreamRetryConfig::from_config(&config).is_err(), "{config}");
        }
    }
}
