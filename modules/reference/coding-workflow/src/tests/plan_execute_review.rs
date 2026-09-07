use super::*;

#[test]
fn plan_execute_review_runs_plan_execute_and_review_requests() {
    let mut input = workflow_input("change code");
    input.runtime.reasoning = ReasoningConfig::new(Some("high".to_owned()), true);
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut host = FakeHost::with_responses(vec![
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "plan"),
            Vec::new(),
            FinishReason::Stop,
        ),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "draft"),
            Vec::new(),
            FinishReason::Stop,
        ),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "final"),
            Vec::new(),
            FinishReason::Stop,
        ),
    ]);
    let host_to = &mut host;

    let output_json =
        match CodingPlanExecuteReviewWorkflow.run_json(String::from(input_json), host_to) {
            Ok(json) => json,
            Err(error) => panic!("workflow failed: {}", error.message),
        };
    let output: WorkflowModuleOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    let _ = host_to;

    assert_eq!(output.output.text, "final");
    assert_eq!(
        output.output.metadata["workflow"]["module_id"],
        PLAN_EXECUTE_REVIEW_MODULE_ID
    );
    assert_eq!(
        output.output.metadata["phases"],
        json!(["plan", "execute", "review"])
    );
    let persisted = output
        .new_messages
        .iter()
        .map(|message| (message.role.clone(), message_text(message)))
        .collect::<Vec<_>>();
    assert_eq!(
        persisted,
        vec![(MessageRole::Assistant, "final".to_owned())]
    );
    assert!(
        output
            .new_messages
            .iter()
            .all(|message| message.metadata["workflow_phase"] != "plan")
    );

    let requests = host.requests.lock().expect("requests");
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].tool_choice, ToolChoice::Auto);
    assert_eq!(
        requests[0].reasoning,
        ReasoningConfig::new(Some("high".to_owned()), true)
    );
    assert!(
        requests[0]
            .tools
            .iter()
            .all(|tool| matches!(tool.safety, ToolSafety::ReadOnly))
    );
    assert_eq!(requests[2].tool_choice, ToolChoice::None);
    assert_eq!(requests[2].tools.len(), 0);

    let compactions = host.compactions.lock().expect("compactions");
    assert_eq!(compactions.len(), 3);
    assert_eq!(compactions[2].reason.as_deref(), Some("review"));
    assert_eq!(compactions[2].window_tokens, Some(16_000));
    assert!(
        compactions[2]
            .request
            .messages
            .iter()
            .any(|message| message_text(message) == "draft")
    );
}

#[test]
fn plan_execute_review_executes_read_only_plan_tool_calls_before_execute() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let plan_call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![
        tool_call_response(plan_call),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "plan"),
            Vec::new(),
            FinishReason::Stop,
        ),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "draft"),
            Vec::new(),
            FinishReason::Stop,
        ),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "final"),
            Vec::new(),
            FinishReason::Stop,
        ),
    ])
    .with_tools(vec![read_file.clone()], vec![read_file]);
    let host_to = &mut host;

    let output_json =
        match CodingPlanExecuteReviewWorkflow.run_json(String::from(input_json), host_to) {
            Ok(json) => json,
            Err(error) => panic!("workflow failed: {}", error.message),
        };
    let output: WorkflowModuleOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    let _ = host_to;

    let executed = host.executed_calls.lock().expect("executed calls");
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].name, "read_file");
    drop(executed);

    assert_eq!(output.output.text, "final");
    assert_eq!(output.output.metadata["plan_tool_rounds_used"], json!(1));

    // Tool result plan-фазы виден execute-фазе в следующем model request.
    let requests = host.requests.lock().expect("requests");
    assert_eq!(requests.len(), 4);
    let execute_request = &requests[2];
    assert!(
        execute_request.messages.iter().any(|message| {
            message.parts.iter().any(|part| {
                matches!(
                    &part.payload,
                    ContentPart::ToolResult { result } if result.output.contains("read_file ok")
                )
            })
        }),
        "plan tool result must be visible to the execute phase"
    );

    // Plan tool call и его результат сохраняются в persistent messages.
    assert!(
        output
            .new_messages
            .iter()
            .any(|message| message.role == MessageRole::Tool)
    );
}

#[test]
fn plan_execute_review_errors_when_plan_calls_non_readonly_tool() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let apply_patch = test_tool("apply_patch", "Apply patch", ToolSafety::WritesFiles);
    let call = ToolCall::new(
        new_call_id(),
        "apply_patch",
        json!({ "patch": "*** Begin Patch\n*** End Patch" }),
    );
    let mut host = FakeHost::with_responses(vec![tool_call_response(call)])
        .with_tools(vec![read_file, apply_patch.clone()], vec![apply_patch]);
    let host_to = &mut host;

    let error = match CodingPlanExecuteReviewWorkflow.run_json(String::from(input_json), host_to) {
        Ok(_) => panic!("workflow unexpectedly succeeded"),
        Err(error) => error,
    };
    let _ = host_to;

    assert!(
        error
            .message
            .as_str()
            .contains("plan model requested tool 'apply_patch' that was not present")
    );
    assert_no_executed_calls(&host);
}

#[test]
fn plan_execute_review_errors_when_execute_calls_unrequested_tool() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let apply_patch = test_tool("apply_patch", "Apply patch", ToolSafety::WritesFiles);
    let call = ToolCall::new(
        new_call_id(),
        "apply_patch",
        json!({ "patch": "*** Begin Patch\n*** End Patch" }),
    );
    let mut host = FakeHost::with_responses(vec![
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "plan"),
            Vec::new(),
            FinishReason::Stop,
        ),
        tool_call_response(call),
    ])
    .with_tools(vec![read_file.clone(), apply_patch], vec![read_file]);
    let host_to = &mut host;

    let error = match CodingPlanExecuteReviewWorkflow.run_json(String::from(input_json), host_to) {
        Ok(_) => panic!("workflow unexpectedly succeeded"),
        Err(error) => error,
    };
    let _ = host_to;

    assert!(
        error
            .message
            .as_str()
            .contains("execute model requested tool 'apply_patch' that was not present")
    );
    assert_no_executed_calls(&host);
}

#[test]
fn plan_execute_review_errors_when_review_calls_tool() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "plan"),
            Vec::new(),
            FinishReason::Stop,
        ),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "draft"),
            Vec::new(),
            FinishReason::Stop,
        ),
        tool_call_response(call),
    ])
    .with_tools(vec![read_file.clone()], vec![read_file]);
    let host_to = &mut host;

    let error = match CodingPlanExecuteReviewWorkflow.run_json(String::from(input_json), host_to) {
        Ok(_) => panic!("workflow unexpectedly succeeded"),
        Err(error) => error,
    };
    let _ = host_to;

    assert!(
        error
            .message
            .as_str()
            .contains("review model requested tool 'read_file' that was not present")
    );
    assert_no_executed_calls(&host);
}

#[test]
fn plan_execute_review_stops_plan_tool_loop_at_round_limit() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let mut responses = Vec::new();
    for _ in 0..3 {
        responses.push(tool_call_response(ToolCall::new(
            new_call_id(),
            "read_file",
            json!({ "path": "src/lib.rs" }),
        )));
    }
    responses.push(CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "forced plan"),
        Vec::new(),
        FinishReason::Stop,
    ));
    responses.push(CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "draft"),
        Vec::new(),
        FinishReason::Stop,
    ));
    responses.push(CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "final"),
        Vec::new(),
        FinishReason::Stop,
    ));
    let mut host =
        FakeHost::with_responses(responses).with_tools(vec![read_file.clone()], vec![read_file]);
    let host_to = &mut host;

    let output_json =
        match CodingPlanExecuteReviewWorkflow.run_json(String::from(input_json), host_to) {
            Ok(json) => json,
            Err(error) => panic!("workflow failed: {}", error.message),
        };
    let output: WorkflowModuleOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    let _ = host_to;

    // Максимум 3 tool-раунда в plan-фазе; последний plan-запрос идёт без tools.
    let executed = host.executed_calls.lock().expect("executed calls");
    assert_eq!(executed.len(), 3);
    drop(executed);

    let requests = host.requests.lock().expect("requests");
    let last_plan_request = &requests[3];
    assert_eq!(last_plan_request.tool_choice, ToolChoice::None);
    assert!(last_plan_request.tools.is_empty());
    drop(requests);

    assert_eq!(output.output.metadata["plan_tool_rounds_used"], json!(3));
    assert_eq!(output.output.text, "final");
}
