use std::collections::{HashMap, HashSet};

use crate::{
    core::{
        HistoryMutationKind, JournalEntry, JournalProjection, ModelResponseOutcome,
        ToolCallRecordPhase,
    },
    domain::{CallId, PartId, ThreadId, ToolCallResolution, TurnId},
    model_standard::{CanonicalMessage, ContentPart, PartProvenance, PartScope},
};

use super::{
    AppTranscriptMessage, append_transcript_message, append_transcript_tool_call,
    append_transcript_tool_result, finalize_interrupted_tools,
};

/// Строит завершённый web transcript из canonical journal. Active streaming
/// turn проецируется отдельно через in-memory `turn_progress`: здесь records
/// уже являются единственным durable источником фактов.
pub(crate) fn journal_transcript_messages(
    projection: &JournalProjection,
    live_turn_id: Option<TurnId>,
) -> Vec<AppTranscriptMessage> {
    let visibility = TranscriptVisibility::from_projection(projection);
    let mut state = TranscriptProjectionState::default();

    for record in &projection.records {
        match &record.entry {
            JournalEntry::HistoryMutated(mutation) => {
                if mutation.mutation == HistoryMutationKind::Replace
                    && mutation.compaction.is_none()
                {
                    state.reset();
                }
                let hide_compactor_parts = mutation.mutation == HistoryMutationKind::Replace
                    && mutation.compaction.is_some();
                for message in &mutation.messages {
                    if record.turn_id == live_turn_id {
                        state.append_live_user_message(message);
                    } else {
                        state.append_message(message, hide_compactor_parts);
                    }
                }
            }
            JournalEntry::ModelResponseRecorded(response)
                if record.turn_id != live_turn_id
                    && visibility.is_root_record(record.thread_id, record.turn_id)
                    && visibility
                        .visible_model_responses
                        .contains(&response.exchange_id) =>
            {
                if let ModelResponseOutcome::Response { response } = &response.outcome {
                    state.append_message(&response.message, false);
                }
            }
            JournalEntry::ToolCallRecorded(tool)
                if record.turn_id != live_turn_id
                    && visibility.is_root_record(record.thread_id, record.turn_id) =>
            {
                match &tool.phase {
                    ToolCallRecordPhase::Requested => state.append_tool_call(&tool.call),
                    ToolCallRecordPhase::Resolved { resolution } => {
                        state.apply_resolution(&tool.call.id, resolution)
                    }
                    ToolCallRecordPhase::ApprovalRequested { .. } => {}
                }
            }
            JournalEntry::ToolResultRecorded(tool)
                if record.turn_id != live_turn_id
                    && visibility.is_root_record(record.thread_id, record.turn_id) =>
            {
                if state.seen_results.insert(tool.result.call_id.clone()) {
                    append_transcript_tool_result(&mut state.transcript, &tool.result);
                }
            }
            _ => {}
        }
    }

    finalize_interrupted_tools(&mut state.transcript);
    state.transcript
}

#[derive(Default)]
struct TranscriptVisibility {
    root_threads: HashMap<TurnId, ThreadId>,
    visible_model_responses: HashSet<crate::domain::ExchangeId>,
}

impl TranscriptVisibility {
    fn from_projection(projection: &JournalProjection) -> Self {
        let mut root_threads = HashMap::new();
        let mut committed_parts = HashSet::new();
        let mut root_lifecycle_calls = HashSet::new();

        for record in &projection.records {
            if matches!(record.entry, JournalEntry::TurnOpened(_))
                && let Some(turn_id) = record.turn_id
            {
                root_threads.insert(turn_id, record.thread_id);
            }
            if let JournalEntry::HistoryMutated(mutation) = &record.entry {
                for message in &mutation.messages {
                    committed_parts.extend(message.parts.iter().map(|part| part.part_id));
                }
            }
        }

        for record in &projection.records {
            if let JournalEntry::ToolCallRecorded(tool) = &record.entry
                && matches!(tool.phase, ToolCallRecordPhase::Requested)
                && is_root_record(&root_threads, record.thread_id, record.turn_id)
            {
                root_lifecycle_calls.insert(tool.call.id.clone());
            }
        }

        let mut visible_model_responses = HashSet::new();
        for record in &projection.records {
            let JournalEntry::ModelResponseRecorded(response) = &record.entry else {
                continue;
            };
            if !is_root_record(&root_threads, record.thread_id, record.turn_id) {
                continue;
            }
            let ModelResponseOutcome::Response { response: model } = &response.outcome else {
                continue;
            };
            let committed = model
                .message
                .parts
                .iter()
                .any(|part| committed_parts.contains(&part.part_id));
            let executed = model
                .tool_calls
                .iter()
                .any(|call| root_lifecycle_calls.contains(&call.id));
            if committed || executed {
                visible_model_responses.insert(response.exchange_id);
            }
        }

        Self {
            root_threads,
            visible_model_responses,
        }
    }

    fn is_root_record(&self, thread_id: ThreadId, turn_id: Option<TurnId>) -> bool {
        is_root_record(&self.root_threads, thread_id, turn_id)
    }
}

fn is_root_record(
    root_threads: &HashMap<TurnId, ThreadId>,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
) -> bool {
    turn_id
        .and_then(|turn_id| root_threads.get(&turn_id))
        .is_some_and(|root_thread_id| *root_thread_id == thread_id)
}

#[derive(Default)]
struct TranscriptProjectionState {
    transcript: Vec<AppTranscriptMessage>,
    seen_parts: HashSet<PartId>,
    seen_calls: HashSet<CallId>,
    seen_results: HashSet<CallId>,
}

impl TranscriptProjectionState {
    fn reset(&mut self) {
        self.transcript.clear();
        self.seen_parts.clear();
        self.seen_calls.clear();
        self.seen_results.clear();
    }

    fn append_message(&mut self, message: &CanonicalMessage, hide_compactor_parts: bool) {
        let mut message = message.clone();
        message.parts.retain(|part| {
            if !self.seen_parts.insert(part.part_id)
                || part.scope != PartScope::Conversation
                || (hide_compactor_parts && part.provenance == PartProvenance::Compactor)
            {
                return false;
            }
            match &part.payload {
                ContentPart::ToolCall { call } => self.seen_calls.insert(call.id.clone()),
                ContentPart::ToolResult { result } => {
                    self.seen_results.insert(result.call_id.clone())
                }
                _ => true,
            }
        });
        if !message.parts.is_empty() {
            append_transcript_message(&mut self.transcript, &message);
        }
    }

    fn append_live_user_message(&mut self, message: &CanonicalMessage) {
        if message.role != crate::model_standard::MessageRole::User {
            return;
        }
        let mut message = message.clone();
        message
            .parts
            .retain(|part| part.provenance == PartProvenance::User);
        self.append_message(&message, false);
    }

    fn append_tool_call(&mut self, call: &crate::domain::ToolCall) {
        if self.seen_calls.insert(call.id.clone()) {
            append_transcript_tool_call(&mut self.transcript, call);
        }
    }

    fn apply_resolution(&mut self, call_id: &str, resolution: &ToolCallResolution) {
        if resolution.permits_side_effect() {
            return;
        }
        let Some(tool) = self
            .transcript
            .iter_mut()
            .rev()
            .filter_map(|message| message.tool.as_mut())
            .find(|tool| tool.call_id == call_id)
        else {
            return;
        };
        let reason = match resolution {
            ToolCallResolution::ApprovalDenied { reason }
            | ToolCallResolution::PolicyDenied { reason }
            | ToolCallResolution::ValidationFailed { reason }
            | ToolCallResolution::Unsupported { reason } => reason.clone(),
            _ => "tool call was not permitted".to_owned(),
        };
        tool.status = "failed".to_owned();
        tool.result = Some(reason);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        core::{
            HistoryMutated, JOURNAL_SCHEMA_VERSION, JournalRecord, ModelRequestRecorded,
            ModelResponseRecorded, ToolCallRecorded, ToolResultRecorded, TurnOpened, TurnSettled,
            TurnSettlementStatus,
        },
        domain::{
            AgentTask, ModelRef, ToolCall, ToolResult, new_exchange_id, new_record_id,
            new_session_id, new_thread_id, new_turn_id,
        },
        model_standard::{
            CanonicalModelRequest, CanonicalModelResponse, FinishReason, MessageRole,
        },
    };

    fn record(
        session_id: crate::domain::SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        session_seq: u64,
        entry: JournalEntry,
    ) -> JournalRecord {
        JournalRecord {
            schema_version: JOURNAL_SCHEMA_VERSION,
            record_id: new_record_id(),
            session_seq,
            timestamp_ms: session_seq as i64,
            session_id,
            thread_id,
            turn_id: Some(turn_id),
            entry,
        }
    }

    #[test]
    fn failed_turn_recovers_model_text_and_terminal_tool_card_from_journal() {
        let session_id = new_session_id();
        let thread_id = new_thread_id();
        let turn_id = new_turn_id();
        let exchange_id = new_exchange_id();
        let user = CanonicalMessage::text(MessageRole::User, "inspect");
        let call = ToolCall::new("call-1", "shell", json!({"command": "ls"}));
        let response_message = CanonicalMessage::new(
            MessageRole::Assistant,
            vec![
                ContentPart::Text {
                    text: "checking".to_owned(),
                },
                ContentPart::ToolCall { call: call.clone() },
            ],
        );
        let response = CanonicalModelResponse::new(
            response_message,
            vec![call.clone()],
            FinishReason::ToolCalls,
        );
        let request =
            CanonicalModelRequest::new(ModelRef::new("fake", "model"), vec![user.clone()]);
        let result = ToolResult::ok(call.id.clone(), "listing");
        let records = vec![
            record(
                session_id,
                thread_id,
                turn_id,
                1,
                JournalEntry::TurnOpened(TurnOpened {
                    task: AgentTask::new("inspect", "/tmp/workspace".into()),
                    base_history_revision: 0,
                    module_epoch: 0,
                    config_snapshot: json!({}),
                }),
            ),
            record(
                session_id,
                thread_id,
                turn_id,
                2,
                JournalEntry::HistoryMutated(HistoryMutated {
                    previous_revision: 0,
                    new_revision: 1,
                    mutation: HistoryMutationKind::Append,
                    messages: vec![user],
                    compaction: None,
                }),
            ),
            record(
                session_id,
                thread_id,
                turn_id,
                3,
                JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                    exchange_id,
                    request,
                }),
            ),
            record(
                session_id,
                thread_id,
                turn_id,
                4,
                JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                    exchange_id,
                    outcome: ModelResponseOutcome::Response { response },
                }),
            ),
            record(
                session_id,
                thread_id,
                turn_id,
                5,
                JournalEntry::ToolCallRecorded(ToolCallRecorded {
                    call: call.clone(),
                    phase: ToolCallRecordPhase::Requested,
                }),
            ),
            record(
                session_id,
                thread_id,
                turn_id,
                6,
                JournalEntry::ToolCallRecorded(ToolCallRecorded {
                    call,
                    phase: ToolCallRecordPhase::Resolved {
                        resolution: ToolCallResolution::Allowed,
                    },
                }),
            ),
            record(
                session_id,
                thread_id,
                turn_id,
                7,
                JournalEntry::ToolResultRecorded(ToolResultRecorded { result }),
            ),
            record(
                session_id,
                thread_id,
                turn_id,
                8,
                JournalEntry::TurnSettled(TurnSettled {
                    status: TurnSettlementStatus::Error,
                    output: None,
                    error: Some("workflow failed after tool execution".to_owned()),
                }),
            ),
        ];
        let live_projection =
            JournalProjection::build(session_id, records[..7].to_vec()).expect("live projection");
        let live_transcript = journal_transcript_messages(&live_projection, Some(turn_id));
        assert_eq!(live_transcript.len(), 1);
        assert_eq!(live_transcript[0].role, "user");
        assert_eq!(live_transcript[0].text, "inspect");

        let projection = JournalProjection::build(session_id, records).expect("projection");

        let transcript = journal_transcript_messages(&projection, None);

        assert!(
            transcript
                .iter()
                .any(|message| message.role == "user" && message.text == "inspect")
        );
        assert!(
            transcript
                .iter()
                .any(|message| message.role == "assistant" && message.text == "checking")
        );
        let tool = transcript
            .iter()
            .find_map(|message| message.tool.as_ref())
            .expect("tool card");
        assert_eq!(tool.call_id, "call-1");
        assert_eq!(tool.status, "done");
        assert_eq!(tool.result.as_deref(), Some("listing"));
    }
}
