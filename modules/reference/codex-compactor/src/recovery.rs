//! Local summary retry and context-window recovery at the compactor boundary.
use crate::summary::{ensure_not_cancelled, try_model_summary};
use proteus_contracts::{
    contracts::CompactionInput,
    model_standard::{CanonicalMessage, ContentPart, ModelFailureKind},
    process_module::{CompactorModuleHostMut, ProcessModuleError},
};
use rand::Rng;
use std::time::Duration;

pub(crate) fn complete_summary_with_recovery(
    input: &CompactionInput,
    mut history: Vec<CanonicalMessage>,
    host: &mut CompactorModuleHostMut<'_>,
) -> Result<String, ProcessModuleError> {
    let max_retries = crate::config::CompactorConfig::parse(&input.config)
        .map_err(ProcessModuleError::new)?
        .stream_max_retries;
    let mut retries = 0;
    loop {
        match try_model_summary(input, &history, host) {
            Ok(summary) => return Ok(summary),
            Err(error) => match error.model_failure.as_ref().map(|failure| failure.kind) {
                Some(ModelFailureKind::Interrupted | ModelFailureKind::SessionBudgetExceeded) => {
                    return Err(error);
                }
                Some(ModelFailureKind::ContextWindowExceeded) => {
                    if history.is_empty() {
                        return Err(error);
                    }
                    remove_first_message_and_counterpart(&mut history);
                    retries = 0;
                }
                Some(ModelFailureKind::Other | ModelFailureKind::StreamDisconnected) | None
                    if retries < max_retries =>
                {
                    retries += 1;
                    wait_for_retry(host, retry_delay(retries))?;
                }
                Some(ModelFailureKind::Other) | Some(_) | None => return Err(error),
            },
        }
    }
}

// Pinned Codex uses a 200ms exponential reconnect backoff (factor 2) plus a
// random multiplier in [0.9, 1.1).
fn retry_delay(attempt: u64) -> Duration {
    let exponent = attempt.saturating_sub(1).min(63);
    let base = 200u64.saturating_mul(1u64 << exponent);
    let jitter = rand::rng().random_range(0.9..1.1);
    Duration::from_millis((base as f64 * jitter) as u64)
}

fn wait_for_retry(
    host: &mut CompactorModuleHostMut<'_>,
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

/// Equivalent to Codex `ContextManager::remove_first_item` for the canonical
/// message form: dropping a call also drops its result, and vice versa. A
/// canonical message can group a batch, so counterparts are removed at part
/// granularity to keep unrelated calls in the same assistant item intact.
fn remove_first_message_and_counterpart(history: &mut Vec<CanonicalMessage>) {
    let removed = history.remove(0);
    let mut call_ids = removed.tool_call_id.into_iter().collect::<Vec<_>>();
    call_ids.extend(removed.parts.iter().filter_map(part_call_id));
    if call_ids.is_empty() {
        return;
    }
    history.retain_mut(|message| {
        let standalone_counterpart = message
            .tool_call_id
            .as_ref()
            .is_some_and(|id| call_ids.iter().any(|removed_id| removed_id == id));
        if standalone_counterpart {
            return false;
        }
        message.parts.retain(|part| {
            !part_call_id(part)
                .as_ref()
                .is_some_and(|id| call_ids.iter().any(|removed_id| removed_id == id))
        });
        !message.parts.is_empty()
    });
}

fn part_call_id(part: &proteus_contracts::model_standard::CanonicalPart) -> Option<String> {
    match &part.payload {
        ContentPart::ToolCall { call } => Some(call.id.clone()),
        ContentPart::ToolResult { result } => Some(result.call_id.clone()),
        _ => None,
    }
}
