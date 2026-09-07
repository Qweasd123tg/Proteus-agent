use super::*;
use crate::domain::{ToolResult, new_session_id, new_thread_id};
use crate::model_standard::MessagePhase;
use proteus_core::core::SessionStore;

#[test]
fn text_grouping_keeps_message_boundaries_and_interleaved_items() {
    let first = CanonicalMessage::new(
        MessageRole::Assistant,
        vec![
            ContentPart::Text {
                text: "before".to_owned(),
            },
            ContentPart::Reasoning {
                text: "summary".to_owned(),
                signature: Some("opaque".to_owned()),
            },
            ContentPart::Text {
                text: "after".to_owned(),
            },
            ContentPart::Text {
                text: "tail".to_owned(),
            },
        ],
    )
    .with_phase(MessagePhase::Commentary);
    let second = CanonicalMessage::text(MessageRole::Assistant, "separate")
        .with_phase(MessagePhase::FinalAnswer);
    let body = to_openai_request(&CanonicalModelRequest::new(
        ModelRef::new("openai", "fixture"),
        vec![first, second],
    ))
    .unwrap();
    let items = body["input"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0]["content"][0]["text"], "before");
    assert_eq!(items[1]["type"], "reasoning");
    assert_eq!(
        items[2]["content"],
        json!([
            {"type": "output_text", "text": "after"}, {"type": "output_text", "text": "tail"}
        ])
    );
    assert_eq!(items[2]["phase"], "commentary");
    assert_eq!(items[3]["phase"], "final_answer");
    assert_eq!(items[3]["content"][0]["text"], "separate");
}

#[tokio::test]
async fn multipart_message_and_custom_tool_survive_journal_round_trip() {
    // Responses messages contain an ordered content array, not one message per part.
    // https://developers.openai.com/api/reference/typescript/resources/responses
    let message = json!({
        "type": "message", "role": "assistant", "phase": "commentary",
        "content": [
            {"type": "output_text", "text": "Первая часть."},
            {"type": "output_text", "text": "\nВторая часть."}
        ]
    });
    let input = "*** Begin Patch\n*** Add File: probe.txt\n+Привет\n*** End Patch\n";
    let custom = json!({
        "type": "custom_tool_call", "call_id": "call_patch", "name": "apply_patch",
        "input": input
    });
    let response = from_openai_response(json!({
        "status": "completed", "output": [message.clone(), custom.clone()]
    }))
    .expect("provider response");
    let mut history = response.messages;
    history.push(CanonicalMessage::new(
        MessageRole::Tool,
        vec![ContentPart::ToolResult {
            result: ToolResult::ok("call_patch".to_owned(), "applied"),
        }],
    ));
    let root = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let store = SessionStore::new(root.path(), workspace.path(), new_session_id()).unwrap();
    store
        .append_history(new_thread_id(), None, &history)
        .await
        .unwrap();
    let session_dir = store.session_dir().to_path_buf();
    drop(store);
    let restored = SessionStore::open(session_dir)
        .unwrap()
        .load_messages()
        .unwrap();
    assert_eq!(restored, history);
    let body = to_openai_request(&CanonicalModelRequest::new(
        ModelRef::new("openai", "fixture"),
        restored,
    ))
    .unwrap();
    assert_eq!(
        body["input"],
        json!([
            message, custom,
            {"type": "custom_tool_call_output", "call_id": "call_patch", "output": "applied"}
        ])
    );
}
