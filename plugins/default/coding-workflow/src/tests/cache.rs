use super::*;

fn cache_relevant_input(request: &CanonicalModelRequest) -> Vec<Value> {
    request
        .messages
        .iter()
        .flat_map(|message| {
            message
                .parts
                .iter()
                .map(|part| json!({ "role": message.role, "part": part }))
        })
        .collect()
}

#[test]
fn stable_context_keeps_the_next_turn_wire_input_append_only() {
    let first_input = workflow_input("first question");
    let session_id = first_input.runtime.session_id;
    let first_input_json = serde_json::to_string(&first_input).expect("first input json");
    let mut first_host = FakeHost::default().with_context_text("stable workspace context");
    let mut first_host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut first_host, TD_Opaque);

    let first_output_json = match CodingSingleLoopWorkflow::default()
        .run_json(RString::from(first_input_json), &mut first_host_to)
    {
        RResult::ROk(json) => json,
        RResult::RErr(error) => panic!("first workflow turn failed: {}", error.message),
    };
    let first_output: PluginWorkflowOutput =
        serde_json::from_str(first_output_json.as_str()).expect("first output json");
    drop(first_host_to);
    let first_request = first_host.requests.lock().expect("first requests")[0].clone();

    let mut second_input = workflow_input("second question");
    second_input.history = first_output.messages;
    second_input.runtime.session_id = session_id;
    let second_input_json = serde_json::to_string(&second_input).expect("second input json");
    let mut second_host = FakeHost::default().with_context_text("stable workspace context");
    let mut second_host_to: PluginWorkflowHostMut<'_> =
        PluginWorkflowHost_TO::from_ptr(&mut second_host, TD_Opaque);

    match CodingSingleLoopWorkflow::default()
        .run_json(RString::from(second_input_json), &mut second_host_to)
    {
        RResult::ROk(_) => {}
        RResult::RErr(error) => panic!("second workflow turn failed: {}", error.message),
    }
    drop(second_host_to);
    let second_request = second_host.requests.lock().expect("second requests")[0].clone();

    assert_eq!(first_request.instructions, second_request.instructions);
    assert_eq!(first_request.tools, second_request.tools);
    assert_eq!(
        first_request.metadata["prompt_cache_key"],
        second_request.metadata["prompt_cache_key"]
    );
    let first_wire_input = cache_relevant_input(&first_request);
    let second_wire_input = cache_relevant_input(&second_request);
    assert_eq!(
        first_wire_input,
        second_wire_input[..first_wire_input.len()],
        "the next turn must extend the provider-visible input prefix"
    );
}
