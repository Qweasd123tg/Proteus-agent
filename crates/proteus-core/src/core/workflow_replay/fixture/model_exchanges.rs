use std::collections::HashMap;

use anyhow::{Result, anyhow, ensure};

use crate::{
    contracts::ModelCallOrigin,
    core::{HistoryMutationKind, JournalEntry, JournalRecord, ModelResponseOutcome},
    domain::{ExchangeId, ExecutionId, ThreadId, TurnId},
    model_standard::CanonicalModelRequest,
};

use super::RecordedModelExchange;

struct PendingExchange {
    exchange_id: ExchangeId,
    origin: ModelCallOrigin,
    request: CanonicalModelRequest,
    outcome: Option<ModelResponseOutcome>,
    completed_messages: Vec<crate::model_standard::CanonicalMessage>,
}

pub(super) fn select_exchanges(
    records: &[JournalRecord],
    execution_id: ExecutionId,
    thread_id: ThreadId,
    turn_id: TurnId,
) -> Result<Vec<RecordedModelExchange>> {
    let mut exchanges = Vec::<PendingExchange>::new();
    let mut positions = HashMap::new();
    for record in records.iter().filter(|record| {
        record.execution_id == Some(execution_id) && record.thread_id == Some(thread_id)
    }) {
        match &record.entry {
            JournalEntry::ModelRequestRecorded(request) => {
                positions.insert(request.exchange_id, exchanges.len());
                exchanges.push(PendingExchange {
                    exchange_id: request.exchange_id,
                    origin: request.origin,
                    request: request.request.clone(),
                    outcome: None,
                    completed_messages: Vec::new(),
                });
            }
            JournalEntry::ModelMessageRecorded(item) => {
                let index = *positions
                    .get(&item.exchange_id)
                    .ok_or_else(|| anyhow!("completed model item has no root request"))?;
                exchanges[index]
                    .completed_messages
                    .push(item.message.clone());
            }
            JournalEntry::ModelResponseRecorded(response) => {
                let index = positions
                    .get(&response.exchange_id)
                    .copied()
                    .ok_or_else(|| {
                        anyhow!(
                            "model response {} has no selected root request",
                            response.exchange_id
                        )
                    })?;
                exchanges[index].outcome = Some(response.outcome.clone());
            }
            _ => {}
        }
    }
    // Validate every exchange before excluding nested model work. A pending
    // compactor call must not disappear into a seemingly model-free turn.
    let exchanges = exchanges
        .into_iter()
        .map(|exchange| {
            Ok((exchange.origin, RecordedModelExchange {
                completed_messages: exchange.completed_messages,
                exchange_id: exchange.exchange_id,
                request: exchange.request,
                outcome: exchange.outcome.ok_or_else(|| {
                    anyhow!("model exchange {} is incomplete and cannot be used for workflow replay", exchange.exchange_id)
                })?,
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    let origins = exchanges
        .iter()
        .map(|(origin, exchange)| (exchange.exchange_id, *origin))
        .collect();
    validate_compactor_boundaries(records, thread_id, turn_id, &origins)?;
    Ok(exchanges
        .into_iter()
        .filter_map(|(origin, exchange)| (origin == ModelCallOrigin::Direct).then_some(exchange))
        .collect())
}

fn validate_compactor_boundaries(
    records: &[JournalRecord],
    thread_id: ThreadId,
    turn_id: TurnId,
    origins: &HashMap<ExchangeId, ModelCallOrigin>,
) -> Result<()> {
    let mut pending = None;
    let mut checkpointed = false;
    for record in records
        .iter()
        .filter(|record| record.thread_id == Some(thread_id) && record.turn_id == Some(turn_id))
    {
        match &record.entry {
            JournalEntry::ModelRequestRecorded(request) => {
                match origins.get(&request.exchange_id) {
                    Some(ModelCallOrigin::Compactor) => {
                        pending = Some(request.exchange_id);
                        checkpointed = false;
                    }
                    Some(ModelCallOrigin::Direct) => {
                        if let Some(exchange_id) = pending.take() {
                            ensure!(
                                checkpointed,
                                "compactor exchange {exchange_id} has no recorded changed-compaction checkpoint before the next direct model request; this workflow replay is not supported"
                            );
                        }
                        checkpointed = false;
                    }
                    None => {}
                }
            }
            JournalEntry::HistoryMutated(mutation)
                if pending.is_some()
                    && mutation.mutation == HistoryMutationKind::Checkpoint
                    && mutation
                        .compaction
                        .as_ref()
                        .is_some_and(|report| report.changed) =>
            {
                checkpointed = true;
            }
            _ => {}
        }
    }
    ensure!(
        pending.is_none(),
        "compactor model exchanges require a recorded changed-compaction checkpoint and a following direct model request for workflow replay"
    );
    Ok(())
}
