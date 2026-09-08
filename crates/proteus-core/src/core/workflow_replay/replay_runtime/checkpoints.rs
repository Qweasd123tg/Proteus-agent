use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use anyhow::{Result, ensure};
use async_trait::async_trait;

use super::{ReplayState, messages_equal};
use crate::{
    contracts::{WorkflowHistoryCheckpoint, WorkflowHistoryRecorder},
    core::{
        HistoryMutationKind, JournalEntry, JournalRecord, ToolCallRecordPhase,
        prepare_failed_history_update,
    },
    domain::{CallId, ExchangeId, ThreadId, TurnId},
    model_standard::CanonicalMessage,
};

#[derive(Debug, Clone)]
pub(crate) struct RecordedCheckpoint {
    messages: Vec<CanonicalMessage>,
    calls: Vec<CallId>,
    position: (usize, usize, usize),
}

pub(crate) fn recorded_checkpoints(
    records: &[JournalRecord],
    thread: ThreadId,
    turn: TurnId,
    direct_exchange_ids: &HashSet<ExchangeId>,
) -> Vec<RecordedCheckpoint> {
    let mut position = (0, 0, 0);
    let mut checkpoints = Vec::new();
    for record in records
        .iter()
        .filter(|record| record.thread_id == Some(thread) && record.turn_id == Some(turn))
    {
        match &record.entry {
            JournalEntry::ModelResponseRecorded(response)
                if direct_exchange_ids.contains(&response.exchange_id) =>
            {
                position.0 += 1
            }
            JournalEntry::ToolCallRecorded(tool)
                if tool.phase == ToolCallRecordPhase::Requested =>
            {
                position.1 += 1
            }
            JournalEntry::ToolResultRecorded(_) => position.2 += 1,
            JournalEntry::HistoryMutated(mutation)
                if mutation.mutation == HistoryMutationKind::Checkpoint =>
            {
                checkpoints.push(RecordedCheckpoint {
                    messages: mutation.messages.clone(),
                    calls: mutation
                        .tool_results
                        .iter()
                        .map(|binding| binding.call_id.clone())
                        .collect(),
                    position,
                })
            }
            _ => {}
        }
    }
    checkpoints
}

pub(crate) struct ReplayCheckpointRecorder {
    state: Arc<ReplayState>,
    initial: Vec<CanonicalMessage>,
    expected: Mutex<VecDeque<RecordedCheckpoint>>,
}

impl ReplayCheckpointRecorder {
    pub(crate) fn new(
        state: Arc<ReplayState>,
        initial: Vec<CanonicalMessage>,
        expected: Vec<RecordedCheckpoint>,
    ) -> Self {
        Self {
            state,
            initial,
            expected: Mutex::new(expected.into()),
        }
    }

    pub(crate) fn finish(&self) {
        let remaining = self.expected.lock().unwrap().len();
        if remaining != 0 {
            self.state.lock().issues.push(format!(
                "workflow omitted {remaining} recorded history checkpoints"
            ));
        }
    }
}

#[async_trait]
impl WorkflowHistoryRecorder for ReplayCheckpointRecorder {
    async fn checkpoint(&self, checkpoint: WorkflowHistoryCheckpoint) -> Result<()> {
        let result = (|| {
            let expected = self.expected.lock().unwrap().pop_front().ok_or_else(|| {
                anyhow::anyhow!("workflow added an unrecorded history checkpoint")
            })?;
            let progress = checkpoint.history;
            let prepared = prepare_failed_history_update(
                &self.initial,
                self.initial
                    .last()
                    .ok_or_else(|| anyhow::anyhow!("missing replay user"))?,
                &progress.new_messages,
                progress.history_replacement.as_deref(),
                &progress.compactions,
                &HashSet::new(),
            )?;
            crate::core::session_journal::history_capture::HistoryCapture::new(
                &prepared.final_messages,
                &checkpoint.tool_results,
            )?;
            let mut inner = self.state.lock();
            ensure!(
                checkpoint.tool_results.len() == expected.calls.len(),
                "checkpoint changed the captured tool result set"
            );
            for (binding, expected_call) in checkpoint.tool_results.iter().zip(&expected.calls) {
                if let Some(mapped) = inner.actual_to_expected.get(&binding.call_id) {
                    ensure!(
                        mapped == expected_call,
                        "checkpoint changed a tool call identity"
                    );
                } else {
                    ensure!(
                        !inner.expected_to_actual.contains_key(expected_call),
                        "checkpoint reused a tool call identity"
                    );
                    inner
                        .actual_to_expected
                        .insert(binding.call_id.clone(), expected_call.clone());
                    inner
                        .expected_to_actual
                        .insert(expected_call.clone(), binding.call_id.clone());
                }
            }
            let position = (
                inner.next_exchange,
                inner.tools.iter().filter(|tool| tool.requested).count(),
                inner
                    .tools
                    .iter()
                    .filter(|tool| tool.result_recorded)
                    .count(),
            );
            ensure!(
                position == expected.position,
                "checkpoint moved across a model/tool boundary: expected {:?}, found {:?}",
                expected.position,
                position
            );
            ensure!(
                messages_equal(
                    &prepared.final_messages,
                    &expected.messages,
                    &inner.actual_to_expected
                ),
                "checkpoint history differs from its recorded snapshot"
            );
            Ok(())
        })();
        if let Err(error) = &result {
            self.state
                .lock()
                .issues
                .push(format!("history checkpoint: {error:#}"));
        }
        result
    }
}
