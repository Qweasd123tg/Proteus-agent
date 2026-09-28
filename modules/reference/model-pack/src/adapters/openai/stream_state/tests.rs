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
        assert!(
            state.translate("{}").is_empty(),
            "missing JSON type is not a Responses envelope"
        );
        let mut payload =
            json!({"type": "response.output_item.added", "output_index": 2, "item": item});
        assert!(state.translate(&payload.to_string()).is_empty());
        payload["type"] = json!("response.output_item.done");
        let events = state.translate(&payload.to_string());
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
            &json!({"type": "response.completed", "response": {"output": []}}).to_string(),
        );
        let [ModelStreamEvent::Response { response }] = terminal.as_slice() else {
            panic!("terminal response")
        };
        assert_eq!(response.messages[0].id, message.id);
        assert_eq!(response.tool_calls, [call.clone()]);
    }
}
