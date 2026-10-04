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

#[test]
fn refusal_delta_done_and_terminal_preserve_visible_text_and_metadata() {
    let mut state = OpenAiStreamState::default();
    let delta = state.translate(
        &json!({"type":"response.refusal.delta","item_id":"refusal","delta":"Cannot comply."})
            .to_string(),
    );
    assert!(matches!(&delta[0], ModelStreamEvent::TextDelta {text,..} if text == "Cannot comply."));
    assert!(state.translate(&json!({"type":"response.refusal.done","item_id":"refusal","refusal":"Cannot comply."}).to_string()).is_empty());
    let item = json!({"id":"refusal","type":"message","role":"assistant","content":[{"type":"refusal","refusal":"Cannot comply."}]});
    let completed =
        state.translate(&json!({"type":"response.output_item.done","item":item}).to_string());
    let [ModelStreamEvent::MessageCompleted { message }] = completed.as_slice() else {
        panic!("missing completed refusal");
    };
    assert_eq!(message.display_text(), "Cannot comply.");
    assert_eq!(message.metadata["refusal_part_indices"], json!([0]));
    let terminal = state
        .translate(&json!({"type":"response.completed","response":{"output":[item]}}).to_string());
    let [ModelStreamEvent::Response { response }] = terminal.as_slice() else {
        panic!("missing terminal refusal");
    };
    assert_eq!(response.messages[0].id, message.id);
    assert_eq!(response.messages[0].display_text(), "Cannot comply.");
    assert_eq!(response.messages[0].metadata, message.metadata);
}

#[test]
fn refusal_deltas_repair_terminal_output_without_done_items() {
    let mut state = OpenAiStreamState::default();
    state.translate(
        &json!({"type":"response.refusal.delta","item_id":"refusal","delta":"No."}).to_string(),
    );
    let terminal =
        state.translate(&json!({"type":"response.completed","response":{"output":[]}}).to_string());
    let [ModelStreamEvent::Response { response }] = terminal.as_slice() else {
        panic!("missing refusal terminal");
    };
    assert_eq!(response.messages[0].display_text(), "No.");
    assert_eq!(
        response.messages[0].metadata["refusal_part_indices"],
        json!([0])
    );
}

#[tokio::test]
async fn empty_reasoning_does_not_create_a_phantom_completed_prefix_through_bound_model() {
    use crate::{domain::ModelRef, model_standard::CanonicalModelRequest};
    use proteus_contracts::contracts::{CancellationToken, ExecutionScope, ProcessModelDescriptor};
    use proteus_core::core::{
        AppConfig, HeadlessApprovalTransport, ModelExecutionBinding, RuntimeRegistry,
    };
    use std::sync::Arc;
    for last in [
        json!({"id":"text","type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}),
        json!({"id":"tool","type":"function_call","name":"read_file","call_id":"call","arguments":"{}"}),
    ] {
        for reasoning in [
            json!({"id":"reasoning","type":"reasoning","summary":[]}),
            json!({"id":"reasoning","type":"reasoning","summary":[],"encrypted_content":"opaque"}),
            json!({"id":"reasoning","type":"reasoning","summary":[{"type":"summary_text","text":"thinking"}]}),
        ] {
            let mut state = OpenAiStreamState::default();
            let mut events = state.translate(
                &json!({"type":"response.output_item.done","output_index":0,"item":reasoning})
                    .to_string(),
            );
            let represented = reasoning["encrypted_content"].is_string()
                || !reasoning["summary"].as_array().unwrap().is_empty();
            assert_eq!(events.len(), usize::from(represented));
            events.extend(
                state.translate(
                    &json!({"type":"response.output_item.done","output_index":1,"item":last})
                        .to_string(),
                ),
            );
            events.extend(
                state.translate(
                    &json!({"type":"response.completed","response":{"output":[reasoning,last]}})
                        .to_string(),
                ),
            );
            let ModelStreamEvent::Response { response } = events.pop().unwrap() else {
                panic!("terminal missing");
            };
            let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../crates/proteus-core/tests/fixtures/process_model.py");
            let config: AppConfig = serde_json::from_value(json!({
                "active_provider":"fixture", "providers":{"fixture":{"provider":"translated","model":"fixture","stream":true}},
                "components":{"translated":{"command":"python3","args":["-B",fixture],"exports":{"model":{"translated":{}}}}},
                "module_config":{"model":{"translated":{
                    "descriptor":ProcessModelDescriptor { adapter_id:"translated".into(), capabilities:crate::model_standard::ModelCapabilities::basic_text_and_tools().with_streaming(true),hosted_tools:vec![] },
                    "events":events,"terminal":{"kind":"response","response":response}
                }}}
            })).unwrap();
            let workspace = tempfile::tempdir().unwrap();
            let registry = RuntimeRegistry::from_config(&config, workspace.path().into()).unwrap();
            let execution = registry
                .execution_context(
                    ModelExecutionBinding::detached(
                        ExecutionScope::fresh(CancellationToken::new()),
                    ),
                    Arc::new(HeadlessApprovalTransport),
                    crate::domain::PermissionMode::Normal,
                )
                .unwrap();
            let response = execution
                .require_model()
                .unwrap()
                .complete(CanonicalModelRequest::new(
                    ModelRef::new("translated", "fixture"),
                    vec![],
                ))
                .await
                .unwrap();
            assert_eq!(response.messages.len(), 1 + usize::from(represented));
        }
    }
}
