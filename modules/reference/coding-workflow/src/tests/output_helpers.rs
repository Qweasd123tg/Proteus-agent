use super::*;

#[test]
fn empty_text_response_gets_placeholder() {
    let message = CanonicalMessage::new(MessageRole::Assistant, Vec::new());

    assert_eq!(message_text(&message), "<empty model response>");
}

#[test]
fn empty_final_output_falls_back_to_latest_tool_result() {
    let result = ToolResult::new(
        proteus_contracts::domain::new_call_id(),
        false,
        "usage: skatewind --place NAME".to_owned(),
        Vec::new(),
        Some("process exited with code 1".to_owned()),
        json!({}),
    );
    let messages = vec![CanonicalMessage::new(
        MessageRole::Tool,
        vec![ContentPart::ToolResult { result }],
    )];
    let message = CanonicalMessage::new(MessageRole::Assistant, Vec::new());

    let text = output_text(&message, &messages);

    assert!(text.contains("Model returned an empty final response"));
    assert!(text.contains("usage: skatewind --place NAME"));
    assert!(text.contains("process exited with code 1"));
}

#[test]
fn empty_final_output_does_not_fall_back_to_previous_turn_tool_result() {
    let result = ToolResult::new(
        proteus_contracts::domain::new_call_id(),
        false,
        "old turn output".to_owned(),
        Vec::new(),
        Some("old turn error".to_owned()),
        json!({}),
    );
    let history = [CanonicalMessage::new(
        MessageRole::Tool,
        vec![ContentPart::ToolResult { result }],
    )];
    let message = CanonicalMessage::new(MessageRole::Assistant, Vec::new());

    let text = output_text(&message, &history[history.len()..]);

    assert_eq!(text, "<empty model response>");
}

#[test]
fn estimates_tokens_from_text_context_and_tool_results() {
    let result =
        ToolResult::ok(proteus_contracts::domain::new_call_id(), "abcd").with_metadata(json!({}));
    let messages = vec![
        CanonicalMessage::text(MessageRole::User, "abcd"),
        CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }]),
    ];

    assert_eq!(estimate_message_tokens(&messages), Some(4));
}
