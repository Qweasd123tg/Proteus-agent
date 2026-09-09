use super::*;
use crate::model_standard::{ContentPart, ModelStreamEvent};

#[test]
fn completed_tool_items_preserve_call_surface_and_terminal_message_identity() {
    for item in [
        json!({"type": "function_call", "id": "function-item", "call_id": "function-call",
            "name": "shell", "arguments": "{ \"command\": \"true\" }"}),
        json!({"type": "custom_tool_call", "id": "custom-item", "call_id": "custom-call",
            "name": "apply_patch", "input": "*** Begin Patch\n*** End Patch"}),
    ] {
        let mut state = OpenAiStreamState::default();
        let payload = json!({"output_index": 2, "item": item}).to_string();
        assert!(
            state
                .translate("response.output_item.added", &payload)
                .is_empty()
        );
        let events = state.translate("response.output_item.done", &payload);
        let [ModelStreamEvent::MessageCompleted { message }] = events.as_slice() else {
            panic!("expected a complete tool item: {events:?}");
        };
        let [part] = message.parts.as_slice() else {
            panic!("one tool part")
        };
        let ContentPart::ToolCall { call } = &part.payload else {
            panic!("tool call")
        };
        assert_eq!(call.id, item["call_id"].as_str().unwrap());
        if item["type"] == "function_call" {
            assert_eq!(call.raw_arguments.as_deref(), item["arguments"].as_str());
            assert_eq!(call.surface, crate::domain::ToolCallSurface::Function);
        } else {
            assert_eq!(call.args["input"], item["input"]);
            assert_eq!(call.surface, crate::domain::ToolCallSurface::Freeform);
        }
        // Empty terminal output is reconstructed only by the provider adapter.
        let terminal = state.translate(
            "response.completed",
            &json!({"response": {"output": []}}).to_string(),
        );
        let [ModelStreamEvent::Response { response }] = terminal.as_slice() else {
            panic!("terminal response")
        };
        assert_eq!(response.messages[0].id, message.id);
        assert_eq!(response.tool_calls, [call.clone()]);
    }
}
