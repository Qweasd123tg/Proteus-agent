//! Codex 67cc3c3, codex-api/src/sse/responses.rs: time out one parsed
//! eventsource poll. Byte fragments, comments and downstream work are not events.
use std::time::Duration;

use eventsource_stream::Event;
use futures_util::{Stream, StreamExt};
use serde_json::Value;

use crate::model_standard::{ModelFailure, ModelFailureKind};

#[derive(Debug, Clone, Copy)]
pub(super) struct SseIdleTimeout(Duration);

impl SseIdleTimeout {
    pub(super) fn from_config(config: &Value) -> anyhow::Result<Self> {
        let milliseconds = config
            .get("stream_idle_timeout_ms")
            .map(|value| {
                value.as_u64().ok_or_else(|| {
                    anyhow::anyhow!("openai stream_idle_timeout_ms must be a non-negative integer")
                })
            })
            .transpose()?
            .unwrap_or(300_000);
        Ok(Self(Duration::from_millis(milliseconds)))
    }

    pub(super) async fn next<S, E>(
        &self,
        stream: &mut S,
    ) -> Result<Option<Result<Event, E>>, ModelFailure>
    where
        S: Stream<Item = Result<Event, E>> + Unpin,
    {
        tokio::time::timeout(self.0, stream.next())
            .await
            .map_err(|_| {
                ModelFailure::new(
                    ModelFailureKind::StreamDisconnected,
                    "idle timeout waiting for SSE",
                )
            })
    }
}

#[cfg(test)]
#[path = "sse_idle_tests.rs"]
mod tests;
