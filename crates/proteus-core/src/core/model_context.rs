//! Policy-free context accounting facts retained across workflow invocations.
//!
//! The workflow interprets window observations. The host never turns an error
//! into provider usage or chooses a compaction threshold.
use std::collections::HashMap;

use crate::{
    contracts::{ModelCallOrigin, ModelContextObservation},
    core::{HistoryMutationKind, JournalEntry, JournalRecord, ModelResponseOutcome},
    domain::{ExchangeId, TurnId},
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, ModelFailure, ModelFailureKind,
    },
};

pub(crate) mod recorder;

#[derive(Debug, Default)]
pub(crate) struct ModelContextState {
    observations: Vec<ModelContextObservation>,
    requests: HashMap<ExchangeId, (ModelCallOrigin, Option<u32>)>,
}

impl ModelContextState {
    pub(crate) fn snapshot(&self) -> Vec<ModelContextObservation> {
        self.observations.clone()
    }

    pub(crate) fn request(
        &mut self,
        exchange: ExchangeId,
        origin: ModelCallOrigin,
        request: &CanonicalModelRequest,
    ) {
        self.requests
            .insert(exchange, (origin, request.limits.max_input_tokens));
    }

    pub(crate) fn response(&mut self, exchange: ExchangeId, response: &CanonicalModelResponse) {
        if self.requests.remove(&exchange).is_none() {
            return;
        }
        let Some(usage) = &response.usage else { return };
        let tokens = u64::from(usage.input_tokens) + u64::from(usage.output_tokens);
        let last = tokens.min(u64::from(u32::MAX)) as u32;
        if let Some(ModelContextObservation::Usage {
            total_tokens,
            last_tokens,
        }) = self.observations.last_mut()
        {
            *total_tokens = total_tokens.saturating_add(tokens);
            *last_tokens = last;
        } else {
            self.observations.push(ModelContextObservation::Usage {
                total_tokens: tokens,
                last_tokens: last,
            });
        }
    }

    pub(crate) fn failure(&mut self, exchange: ExchangeId, failure: &ModelFailure) {
        let Some((ModelCallOrigin::Direct, Some(max_input_tokens))) =
            self.requests.remove(&exchange)
        else {
            return;
        };
        if failure.kind == ModelFailureKind::ContextWindowExceeded {
            self.observations
                .push(ModelContextObservation::ContextWindowExceeded { max_input_tokens });
        }
    }

    pub(crate) fn compacted(&mut self) {
        if !matches!(
            self.observations.last(),
            Some(ModelContextObservation::HistoryCompacted)
        ) {
            self.observations
                .push(ModelContextObservation::HistoryCompacted);
        }
    }

    /// Build the same facts for cold resume and for the prefix preceding a
    /// replayed turn. Root TurnOpened lineage survives resume with a new thread;
    /// detached and child executions cannot affect the shared conversation.
    pub(crate) fn from_records(records: &[JournalRecord], before_turn: Option<TurnId>) -> Self {
        let mut state = Self::default();
        let mut root_executions = HashMap::new();
        for record in records {
            if matches!(record.entry, JournalEntry::TurnOpened(_))
                && before_turn.is_some_and(|turn| record.turn_id == Some(turn))
            {
                break;
            }
            match &record.entry {
                JournalEntry::TurnOpened(_) => {
                    if let (Some(turn), Some(thread), Some(execution)) =
                        (record.turn_id, record.thread_id, record.execution_id)
                    {
                        root_executions.insert(turn, (thread, execution));
                    }
                }
                JournalEntry::HistoryMutated(mutation) => {
                    if mutation
                        .compaction
                        .as_ref()
                        .is_some_and(|report| report.changed)
                    {
                        state.compacted();
                    } else if mutation.mutation == HistoryMutationKind::Replace {
                        state = Self::default();
                    }
                }
                JournalEntry::ModelRequestRecorded(request)
                    if record
                        .turn_id
                        .and_then(|turn| root_executions.get(&turn).copied())
                        .is_some_and(|owner| {
                            Some(owner) == record.thread_id.zip(record.execution_id)
                        }) =>
                {
                    state.request(request.exchange_id, request.origin, &request.request);
                }
                JournalEntry::ModelResponseRecorded(response)
                    if record
                        .turn_id
                        .and_then(|turn| root_executions.get(&turn).copied())
                        .is_some_and(|owner| {
                            Some(owner) == record.thread_id.zip(record.execution_id)
                        }) =>
                {
                    match &response.outcome {
                        ModelResponseOutcome::Response { response: model } => {
                            state.response(response.exchange_id, model);
                        }
                        ModelResponseOutcome::Error { failure } => {
                            state.failure(response.exchange_id, failure);
                        }
                    }
                }
                _ => {}
            }
        }
        // Incomplete exchanges from a dead process cannot complete in this one.
        state.requests.clear();
        state
    }
}

#[cfg(test)]
mod tests;
