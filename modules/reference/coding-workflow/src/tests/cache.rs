use super::*;

fn cache_relevant_input(request: &CanonicalModelRequest) -> Vec<Value> {
    request
        .messages
        .iter()
        .flat_map(|message| {
            message
                .parts
                .iter()
                .map(|part| json!({ "role": message.role, "part": part.payload }))
        })
        .collect()
}

#[test]
fn stable_context_keeps_the_next_turn_wire_input_append_only() {
    let first_input = workflow_input("first question");
    let session_id = first_input
        .runtime
        .conversation
        .as_ref()
        .unwrap()
        .session_id;
    let first_input_json = serde_json::to_string(&first_input).expect("first input json");
    let mut first_host = FakeHost::default().with_context_text("stable workspace context");

    let first_output_json = CodingSingleLoopWorkflow::default()
        .run_json(first_input_json, &mut first_host)
        .expect("first workflow turn succeeds");
    let first_output: WorkflowModuleOutput =
        serde_json::from_str(first_output_json.as_str()).expect("first output json");
    let first_request = first_host.requests.lock().expect("first requests")[0].clone();

    let mut second_input = workflow_input("second question");
    let second_user = second_input.history.pop().expect("second user message");
    let mut second_history = first_input.history.clone();
    second_history.extend(first_output.new_messages);
    second_history.push(second_user);
    second_input.history = second_history;
    second_input
        .runtime
        .conversation
        .as_mut()
        .unwrap()
        .session_id = session_id;
    let second_input_json = serde_json::to_string(&second_input).expect("second input json");
    let mut second_host = FakeHost::default().with_context_text("stable workspace context");

    CodingSingleLoopWorkflow::default()
        .run_json(second_input_json, &mut second_host)
        .expect("second workflow turn succeeds");
    let second_request = second_host.requests.lock().expect("second requests")[0].clone();

    assert_eq!(first_request.instructions, second_request.instructions);
    assert_eq!(first_request.tools, second_request.tools);
    assert_eq!(
        first_request.cache.routing_key.as_deref(),
        second_request.cache.routing_key.as_deref()
    );
    let first_wire_input = cache_relevant_input(&first_request);
    let second_wire_input = cache_relevant_input(&second_request);
    assert_eq!(
        first_wire_input,
        second_wire_input[..first_wire_input.len()],
        "the next turn must extend the provider-visible input prefix"
    );
}
