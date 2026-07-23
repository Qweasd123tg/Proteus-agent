use serde_json::json;

use crate::{
    domain::{
        AgentTask, ModelRef, ToolCall, ToolResult, new_call_id, new_exchange_id, new_record_id,
        new_session_id, new_thread_id, new_turn_id,
    },
    model_standard::{CanonicalMessage, CanonicalModelRequest, MessageRole},
};

use super::*;

fn record(
    session_id: crate::domain::SessionId,
    thread_id: crate::domain::ThreadId,
    turn_id: Option<crate::domain::TurnId>,
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
        turn_id,
        entry,
    }
}

fn opened(base_history_revision: u64) -> JournalEntry {
    JournalEntry::TurnOpened(TurnOpened {
        task: AgentTask::new("test", std::path::PathBuf::from("/tmp/workspace")),
        base_history_revision,
        module_epoch: 0,
        config_snapshot: json!({}),
    })
}

#[test]
fn projection_rejects_non_monotonic_sequence() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let records = vec![record(session_id, thread_id, Some(turn_id), 2, opened(0))];

    let error = JournalProjection::build(session_id, records).expect_err("sequence gap");

    assert!(error.to_string().contains("expected 1, found 2"));
}

#[test]
fn projection_rejects_unknown_record_schema() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let mut invalid = record(session_id, thread_id, Some(turn_id), 1, opened(0));
    invalid.schema_version = JOURNAL_SCHEMA_VERSION + 1;

    let error = JournalProjection::build(session_id, vec![invalid]).expect_err("unknown schema");

    assert!(
        error
            .to_string()
            .contains("unsupported journal schema_version"),
        "{error:#}"
    );
}

#[test]
fn projection_rejects_history_revision_mismatch() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let mutation = JournalEntry::HistoryMutated(HistoryMutated {
        previous_revision: 7,
        new_revision: 8,
        mutation: HistoryMutationKind::Append,
        messages: vec![CanonicalMessage::text(MessageRole::User, "hello")],
        compaction: None,
    });

    let error = JournalProjection::build(
        session_id,
        vec![record(session_id, thread_id, None, 1, mutation)],
    )
    .expect_err("revision mismatch");

    assert!(
        error
            .to_string()
            .contains("expects 7, current revision is 0")
    );
}

#[test]
fn model_response_requires_matching_request() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let exchange_id = new_exchange_id();
    let records = vec![
        record(session_id, thread_id, Some(turn_id), 1, opened(0)),
        record(
            session_id,
            thread_id,
            Some(turn_id),
            2,
            JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                exchange_id,
                outcome: ModelResponseOutcome::Error {
                    message: "network".to_owned(),
                },
            }),
        ),
    ];

    let error = JournalProjection::build(session_id, records).expect_err("orphan response");

    assert!(error.to_string().contains("has no preceding request"));
}

#[test]
fn model_request_requires_a_previously_opened_turn() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let request = CanonicalModelRequest::new(
        ModelRef::new("fake", "model"),
        vec![CanonicalMessage::text(MessageRole::User, "hello")],
    );
    let records = vec![record(
        session_id,
        thread_id,
        Some(turn_id),
        1,
        JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
            exchange_id: new_exchange_id(),
            request,
        }),
    )];

    let error = JournalProjection::build(session_id, records).expect_err("orphan turn");

    assert!(
        error.to_string().contains("before it was opened"),
        "{error:#}"
    );
}

#[test]
fn model_exchange_can_use_a_child_thread_but_cannot_change_owner() {
    let session_id = new_session_id();
    let root_thread_id = new_thread_id();
    let child_thread_id = new_thread_id();
    let other_thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let exchange_id = new_exchange_id();
    let request = CanonicalModelRequest::new(
        ModelRef::new("fake", "model"),
        vec![CanonicalMessage::text(MessageRole::User, "hello")],
    );
    let response = crate::model_standard::CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "done"),
        Vec::new(),
        crate::model_standard::FinishReason::Stop,
    );
    let prefix = vec![
        record(session_id, root_thread_id, Some(turn_id), 1, opened(0)),
        record(
            session_id,
            child_thread_id,
            Some(turn_id),
            2,
            JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                exchange_id,
                request,
            }),
        ),
    ];
    let mut valid = prefix.clone();
    valid.push(record(
        session_id,
        child_thread_id,
        Some(turn_id),
        3,
        JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
            exchange_id,
            outcome: ModelResponseOutcome::Response {
                response: response.clone(),
            },
        }),
    ));
    JournalProjection::build(session_id, valid).expect("child exchange");

    let mut invalid = prefix;
    invalid.push(record(
        session_id,
        other_thread_id,
        Some(turn_id),
        3,
        JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
            exchange_id,
            outcome: ModelResponseOutcome::Response { response },
        }),
    ));
    let error = JournalProjection::build(session_id, invalid).expect_err("owner changed");

    assert!(
        error.to_string().contains("changed lifecycle owner"),
        "{error:#}"
    );
}

#[test]
fn background_child_exchange_may_finish_after_root_settlement() {
    let session_id = new_session_id();
    let root_thread_id = new_thread_id();
    let child_thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let exchange_id = new_exchange_id();
    let request = CanonicalModelRequest::new(
        ModelRef::new("fake", "model"),
        vec![CanonicalMessage::text(MessageRole::User, "background")],
    );
    let response = crate::model_standard::CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "finished later"),
        Vec::new(),
        crate::model_standard::FinishReason::Stop,
    );
    let records = vec![
        record(session_id, root_thread_id, Some(turn_id), 1, opened(0)),
        record(
            session_id,
            child_thread_id,
            Some(turn_id),
            2,
            JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                exchange_id,
                request,
            }),
        ),
        record(
            session_id,
            root_thread_id,
            Some(turn_id),
            3,
            JournalEntry::TurnSettled(TurnSettled {
                status: TurnSettlementStatus::Success,
                output: None,
                error: None,
            }),
        ),
        record(
            session_id,
            child_thread_id,
            Some(turn_id),
            4,
            JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                exchange_id,
                outcome: ModelResponseOutcome::Response { response },
            }),
        ),
    ];

    JournalProjection::build(session_id, records).expect("background child lifecycle");
}

#[test]
fn request_without_response_and_call_without_result_remain_interrupted() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let exchange_id = new_exchange_id();
    let call = ToolCall::new(new_call_id(), "write_file", json!({"path": "x"}));
    let request = CanonicalModelRequest::new(
        ModelRef::new("fake", "model"),
        vec![CanonicalMessage::text(MessageRole::User, "write")],
    );
    let records = vec![
        record(session_id, thread_id, Some(turn_id), 1, opened(0)),
        record(
            session_id,
            thread_id,
            Some(turn_id),
            2,
            JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                exchange_id,
                request,
            }),
        ),
        record(
            session_id,
            thread_id,
            Some(turn_id),
            3,
            JournalEntry::ToolCallRecorded(ToolCallRecorded {
                call: call.clone(),
                phase: ToolCallRecordPhase::Requested,
            }),
        ),
    ];

    let projection = JournalProjection::build(session_id, records).expect("projection");

    assert_eq!(projection.interrupted_model_exchanges, vec![exchange_id]);
    assert_eq!(projection.unresolved_tool_calls, vec![call.id]);
    assert_eq!(projection.unsettled_turns, vec![turn_id]);
}

#[test]
fn tool_result_must_follow_requested_and_resolved_call() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let call_id = new_call_id();
    let records = vec![
        record(session_id, thread_id, Some(turn_id), 1, opened(0)),
        record(
            session_id,
            thread_id,
            Some(turn_id),
            2,
            JournalEntry::ToolResultRecorded(ToolResultRecorded {
                result: ToolResult::ok(call_id.clone(), "done"),
            }),
        ),
    ];

    let error = JournalProjection::build(session_id, records).expect_err("orphan result");

    assert!(error.to_string().contains("has no preceding call"));
}

#[test]
fn history_rejects_request_scoped_context_parts() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let message = CanonicalMessage::new(
        MessageRole::User,
        vec![crate::model_standard::ContentPart::Context {
            chunk: crate::domain::ContextChunk::new("repo", "context"),
        }],
    );
    let mutation = JournalEntry::HistoryMutated(HistoryMutated {
        previous_revision: 0,
        new_revision: 1,
        mutation: HistoryMutationKind::Append,
        messages: vec![message],
        compaction: None,
    });

    let error = JournalProjection::build(
        session_id,
        vec![record(session_id, thread_id, None, 1, mutation)],
    )
    .expect_err("request scope cannot enter history");

    assert!(error.to_string().contains("non-conversation part"));
}

#[test]
fn active_history_rejects_duplicate_part_ids_across_messages() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let part = CanonicalMessage::text(MessageRole::User, "shared")
        .parts
        .into_iter()
        .next()
        .expect("part");
    let first = CanonicalMessage::from_parts(MessageRole::User, vec![part.clone()]);
    let second = CanonicalMessage::from_parts(MessageRole::User, vec![part]);
    let mutation = JournalEntry::HistoryMutated(HistoryMutated {
        previous_revision: 0,
        new_revision: 1,
        mutation: HistoryMutationKind::Append,
        messages: vec![first, second],
        compaction: None,
    });

    let error = JournalProjection::build(
        session_id,
        vec![record(session_id, thread_id, None, 1, mutation)],
    )
    .expect_err("duplicate part id must fail");

    assert!(error.to_string().contains("duplicate part id"), "{error:#}");
}

#[test]
fn history_replacement_cannot_change_a_recorded_part() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let original = CanonicalMessage::text(MessageRole::User, "original");
    let mut changed_part = original.parts[0].clone();
    changed_part.payload = crate::model_standard::ContentPart::Text {
        text: "changed".to_owned(),
    };
    let changed = CanonicalMessage::from_parts(MessageRole::User, vec![changed_part]);
    let records = vec![
        record(
            session_id,
            thread_id,
            None,
            1,
            JournalEntry::HistoryMutated(HistoryMutated {
                previous_revision: 0,
                new_revision: 1,
                mutation: HistoryMutationKind::Append,
                messages: vec![original],
                compaction: None,
            }),
        ),
        record(
            session_id,
            thread_id,
            None,
            2,
            JournalEntry::HistoryMutated(HistoryMutated {
                previous_revision: 1,
                new_revision: 2,
                mutation: HistoryMutationKind::Replace,
                messages: vec![changed],
                compaction: None,
            }),
        ),
    ];

    let error = JournalProjection::build(session_id, records)
        .expect_err("stable part id cannot change payload");

    assert!(
        error.to_string().contains("changed after it was recorded"),
        "{error:#}"
    );
}
