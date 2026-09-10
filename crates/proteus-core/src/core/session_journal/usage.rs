use std::collections::HashMap;

use super::{JournalEntry, JournalRecord, ModelResponseOutcome, TurnSettlementStatus};
use crate::domain::{
    ExchangeId, ModelRequestUsage, ModelUsageStatus, SessionId, SessionUsageSnapshot, TurnId,
};

/// Компактная проекция: не удерживает тексты prompts, ответов и tools.
/// Обновляется только после успешной durable записи, под mutex writer-а.
#[derive(Debug, Default)]
pub(crate) struct UsageProjection {
    revision: u64,
    latest_turn_id: Option<TurnId>,
    requests: Vec<ModelRequestUsage>,
    pending: HashMap<ExchangeId, usize>,
}

impl UsageProjection {
    pub(crate) fn apply(&mut self, record: &JournalRecord) {
        match &record.entry {
            JournalEntry::TurnOpened(_) => self.latest_turn_id = record.turn_id,
            JournalEntry::ModelRequestRecorded(model) => {
                self.pending.insert(model.exchange_id, self.requests.len());
                self.requests.push(ModelRequestUsage {
                    exchange_id: model.exchange_id,
                    turn_id: record.turn_id,
                    model: model.request.model.clone(),
                    origin: model.origin,
                    started_at_ms: record.timestamp_ms,
                    finished_at_ms: None,
                    status: ModelUsageStatus::Unfinished,
                    finish_reason: None,
                    usage: None,
                    message_count: model.request.messages.len(),
                    tool_count: model.request.tools.len(),
                    reasoning_effort: model.request.reasoning.effort.clone(),
                    max_output_tokens: model.request.limits.max_output_tokens,
                });
            }
            JournalEntry::ModelResponseRecorded(model) => {
                let index = self
                    .pending
                    .remove(&model.exchange_id)
                    .expect("validated model exchange");
                let request = &mut self.requests[index];
                request.finished_at_ms = Some(record.timestamp_ms);
                match &model.outcome {
                    ModelResponseOutcome::Response { response } => {
                        request.status = ModelUsageStatus::Completed;
                        request.usage = response.usage.clone();
                        request.finish_reason = Some(response.finish_reason.clone());
                    }
                    ModelResponseOutcome::Error { .. } => request.status = ModelUsageStatus::Error,
                }
            }
            JournalEntry::TurnSettled(settled) => {
                for index in self.pending.values() {
                    let request = &mut self.requests[*index];
                    if request.turn_id != record.turn_id {
                        continue;
                    }
                    request.status = match settled.status {
                        TurnSettlementStatus::Canceled => ModelUsageStatus::Canceled,
                        TurnSettlementStatus::Timeout => ModelUsageStatus::Timeout,
                        _ => ModelUsageStatus::Unfinished,
                    };
                }
            }
            _ => return,
        }
        self.revision = record.session_seq;
    }

    pub(crate) fn snapshot(&self, session_id: SessionId) -> SessionUsageSnapshot {
        SessionUsageSnapshot {
            session_id,
            revision: self.revision,
            latest_turn_id: self.latest_turn_id,
            requests: self.requests.clone(),
        }
    }
}
