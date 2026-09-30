use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use anyhow::{Result, ensure};
use async_trait::async_trait;

use super::{ReplayState, calls_equal, messages_equal};
use crate::{
    contracts::{
        ExecutionAttribution, ToolExecutionRecorder, WorkflowHistoryCheckpoint,
        WorkflowHistoryRecorder,
    },
    core::{
        HistoryMutationKind, JournalEntry, JournalRecord, ToolCallRecordPhase,
        prepare_failed_history_update,
    },
    domain::{ExchangeId, ThreadId, ToolCall, ToolCallResolution, ToolResult, TurnId},
    model_standard::CanonicalMessage,
};

#[derive(Debug, Clone)]
pub(crate) struct RecordedCheckpoint {
    messages: Vec<CanonicalMessage>,
    calls: Vec<ToolCall>,
    position: (usize, usize, usize),
}

pub(crate) fn recorded_checkpoints(
    records: &[JournalRecord],
    thread: ThreadId,
    turn: TurnId,
    direct_exchange_ids: &HashSet<ExchangeId>,
) -> Vec<RecordedCheckpoint> {
    let mut model_requests = 0;
    let mut requested = HashSet::new();
    let mut results = HashSet::new();
    let mut checkpoints = Vec::new();
    for record in records
        .iter()
        .filter(|record| record.thread_id == Some(thread) && record.turn_id == Some(turn))
    {
        match &record.entry {
            JournalEntry::ModelRequestRecorded(request)
                if direct_exchange_ids.contains(&request.exchange_id) =>
            {
                model_requests += 1
            }
            JournalEntry::ToolCallRecorded(tool)
                if tool.phase == ToolCallRecordPhase::Requested =>
            {
                requested.insert(tool.call.id.clone());
            }
            JournalEntry::ToolResultRecorded(tool) => {
                results.insert(tool.result.call_id.clone());
            }
            JournalEntry::HistoryMutated(mutation)
                if mutation.mutation == HistoryMutationKind::Checkpoint =>
            {
                let pending = mutation
                    .tool_results
                    .iter()
                    .map(|binding| binding.call_id.clone())
                    .collect::<HashSet<_>>();
                checkpoints.push(RecordedCheckpoint {
                    messages: mutation.messages.clone(),
                    calls: mutation
                        .tool_results
                        .iter()
                        .map(|binding| binding.execution_call.clone())
                        .collect(),
                    position: (
                        model_requests,
                        requested.difference(&pending).count(),
                        results.difference(&pending).count(),
                    ),
                })
            }
            _ => {}
        }
    }
    checkpoints
}

struct CommittedHistory {
    history: Vec<CanonicalMessage>,
    capture: crate::core::session_journal::history_capture::HistoryCapture,
    compactions: usize,
}

pub(crate) struct ReplayCheckpointRecorder {
    state: Arc<ReplayState>,
    initial: Vec<CanonicalMessage>,
    expected: Mutex<VecDeque<RecordedCheckpoint>>,
    committed: Mutex<CommittedHistory>,
}

impl ReplayCheckpointRecorder {
    pub(crate) fn new(
        state: Arc<ReplayState>,
        initial: Vec<CanonicalMessage>,
        expected: Vec<RecordedCheckpoint>,
    ) -> Self {
        Self {
            state,
            committed: Mutex::new(CommittedHistory {
                history: initial.clone(),
                capture: Default::default(),
                compactions: 0,
            }),
            initial,
            expected: Mutex::new(expected.into()),
        }
    }

    pub(crate) fn committed_history(&self) -> Vec<CanonicalMessage> {
        self.committed.lock().unwrap().history.clone()
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
                        mapped == &expected_call.id,
                        "checkpoint changed a tool call identity"
                    );
                } else {
                    ensure!(
                        !inner.expected_to_actual.contains_key(&expected_call.id),
                        "checkpoint reused a tool call identity"
                    );
                    inner
                        .actual_to_expected
                        .insert(binding.call_id.clone(), expected_call.id.clone());
                    inner
                        .expected_to_actual
                        .insert(expected_call.id.clone(), binding.call_id.clone());
                }
                ensure!(
                    calls_equal(
                        &binding.execution_call,
                        expected_call,
                        &inner.actual_to_expected
                    ),
                    "checkpoint changed a tool execution binding"
                );
            }
            // A declared capture may still be in flight on either side of a
            // checkpoint. Normalize only these lifecycles; non-pending tools,
            // the exact binding set, snapshots and final drain remain strict.
            let pending = expected
                .calls
                .iter()
                .map(|call| &call.id)
                .collect::<HashSet<_>>();
            let position = (
                inner.next_exchange,
                inner
                    .tools
                    .iter()
                    .filter(|tool| tool.requested && !pending.contains(&tool.recorded.call.id))
                    .count(),
                inner
                    .tools
                    .iter()
                    .filter(|tool| {
                        tool.result_recorded && !pending.contains(&tool.recorded.call.id)
                    })
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
            drop(inner);
            let mut committed = self.committed.lock().unwrap();
            ensure!(
                progress.compactions.len() >= committed.compactions,
                "checkpoint discarded previously reported compactions"
            );
            let compacted = progress
                .compactions
                .iter()
                .skip(committed.compactions)
                .any(|report| report.changed);
            let (capture, history) = committed.capture.rebase(
                &committed.history,
                &prepared.final_messages,
                &checkpoint.tool_results,
                compacted,
            )?;
            committed.history = history;
            committed.capture = capture;
            committed.compactions = progress.compactions.len();
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

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;

#[async_trait]
impl ToolExecutionRecorder for ReplayCheckpointRecorder {
    async fn tool_call_requested(
        &self,
        attribution: ExecutionAttribution,
        call: &ToolCall,
    ) -> Result<()> {
        self.committed.lock().unwrap().capture.validate_call(call)?;
        self.state.tool_call_requested(attribution, call).await
    }
    async fn tool_call_resolved(
        &self,
        attribution: ExecutionAttribution,
        call: &ToolCall,
        resolution: &ToolCallResolution,
    ) -> Result<()> {
        self.state
            .tool_call_resolved(attribution, call, resolution)
            .await
    }
    async fn tool_approval_requested(
        &self,
        attribution: ExecutionAttribution,
        call: &ToolCall,
        reason: &str,
    ) -> Result<()> {
        self.state
            .tool_approval_requested(attribution, call, reason)
            .await
    }
    async fn tool_result_recorded(
        &self,
        attribution: ExecutionAttribution,
        result: &ToolResult,
    ) -> Result<()> {
        self.state.tool_result_recorded(attribution, result).await?;
        let mut committed = self.committed.lock().unwrap();
        let CommittedHistory {
            capture, history, ..
        } = &mut *committed;
        capture.record(history, result)?;
        Ok(())
    }
}
