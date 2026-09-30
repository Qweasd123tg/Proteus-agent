use super::*;
use crate::core::{JOURNAL_SCHEMA_VERSION, ToolResultRecorded};
use crate::domain::{new_execution_id, new_session_id, new_thread_id, new_turn_id};

#[test]
fn final_tool_projection_wins_but_raw_effect_survives_without_final_result() {
    let execution_id = new_execution_id();
    let thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let session_id = new_session_id();
    let call = ToolCall::new("effect", "write_file", serde_json::json!({}));
    let record = |entry| JournalRecord {
        schema_version: JOURNAL_SCHEMA_VERSION,
        record_id: uuid::Uuid::new_v4(),
        session_seq: 0,
        timestamp_ms: 0,
        session_id,
        execution_id: Some(execution_id),
        thread_id: Some(thread_id),
        turn_id: Some(turn_id),
        entry,
    };
    let mut records = vec![
        record(JournalEntry::ToolCallRecorded(
            crate::core::ToolCallRecorded {
                call: call.clone(),
                phase: ToolCallRecordPhase::Requested,
            },
        )),
        record(JournalEntry::ToolCallRecorded(
            crate::core::ToolCallRecorded {
                call: call.clone(),
                phase: ToolCallRecordPhase::Resolved {
                    resolution: ToolCallResolution::Allowed,
                },
            },
        )),
        record(JournalEntry::ToolEffectRecorded(ToolResultRecorded {
            result: ToolResult::ok(call.id.clone(), "actual effect"),
        })),
    ];
    let tools = select_tools(&records, execution_id, thread_id).unwrap();
    assert_eq!(tools[0].result.output, "actual effect");
    assert_eq!(
        tools[0].raw_result.as_ref().unwrap().output,
        "actual effect"
    );
    records.push(record(JournalEntry::ToolResultRecorded(
        ToolResultRecorded {
            result: ToolResult::ok(call.id.clone(), "model-visible output"),
        },
    )));
    let tools = select_tools(&records, execution_id, thread_id).unwrap();
    assert_eq!(tools[0].result.output, "model-visible output");
    assert_eq!(
        tools[0].raw_result.as_ref().unwrap().output,
        "actual effect"
    );
}
