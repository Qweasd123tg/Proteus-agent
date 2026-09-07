use super::*;

#[test]
fn single_loop_calls_host_and_returns_new_messages() {
    let input = workflow_input("hello");
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut host = FakeHost::default();
    let host_to = &mut host;

    let output_json =
        match CodingSingleLoopWorkflow::default().run_json(String::from(input_json), host_to) {
            Ok(json) => json,
            Err(error) => panic!("workflow failed: {}", error.message),
        };
    let output: WorkflowModuleOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    let _ = host_to;

    assert_eq!(output.output.text, "done");
    assert_eq!(
        output.output.metadata["workflow"]["module_id"],
        SINGLE_LOOP_MODULE_ID
    );
    assert_eq!(output.new_messages.len(), 1);

    let requests = host.requests.lock().expect("requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].tools.len(), 0);
    // Потолок окна из runtime должен оказаться в лимитах запроса —
    // иначе снимок TokenUsageUpdated уедет без max_input_tokens.
    assert_eq!(requests[0].limits.max_input_tokens, Some(16_000));
    assert_eq!(requests[0].messages[0].name.as_deref(), Some("context"));
    assert_eq!(requests[0].messages[1].role, MessageRole::User);
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.name.as_deref() == Some("context"))
    );

    let events = host.events.lock().expect("events");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::TaskReceived { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::ContextBuilt { chunks: 1, .. }))
    );
    assert!(events.iter().any(|event| matches!(
        event,
        Event::TokenUsageUpdated {
            usage
        } if usage.categories.iter().any(|category| category.name == "context")
    )));
    // Снимок несёт потолок окна — это знаменатель для бублика контекста в web UI.
    assert!(events.iter().any(|event| matches!(
        event,
        Event::TokenUsageUpdated { usage } if usage.max_input_tokens == Some(16_000)
    )));
    // No-op compactor output does not declare an autocompact trigger, so
    // the UI must not show a fake threshold marker.
    assert!(events.iter().any(|event| matches!(
        event,
        Event::TokenUsageUpdated { usage } if usage.compaction_trigger_tokens.is_none()
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::TurnFinished { .. }))
    );
}

#[test]
fn single_loop_adds_dynamic_meta_tools_when_tool_exposure_hides_candidates() {
    let input = workflow_input("inspect history");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let git_log = test_tool("git_log", "Show commit history", ToolSafety::ReadOnly);
    let mut host =
        FakeHost::default().with_tools(vec![read_file.clone(), git_log], vec![read_file]);
    let host_to = &mut host;

    let output_json =
        match CodingSingleLoopWorkflow::default().run_json(String::from(input_json), host_to) {
            Ok(json) => json,
            Err(error) => panic!("workflow failed: {}", error.message),
        };
    let _output: WorkflowModuleOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    let _ = host_to;

    let requests = host.requests.lock().expect("requests");
    let tool_names = requests[0]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        tool_names,
        vec![
            "read_file",
            dynamic_tools::TOOL_SEARCH,
            dynamic_tools::TOOL_DESCRIBE,
            dynamic_tools::TOOL_CALL,
        ]
    );
    assert!(
        requests[0]
            .instructions
            .iter()
            .any(|instruction| instruction.text.contains("full tool catalog"))
    );
}

#[test]
fn single_loop_errors_when_model_calls_unrequested_tool() {
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
        .with_tools(vec![read_file.clone(), apply_patch], vec![read_file]);
    let host_to = &mut host;

    let error =
        match CodingSingleLoopWorkflow::default().run_json(String::from(input_json), host_to) {
            Ok(_) => panic!("workflow unexpectedly succeeded"),
            Err(error) => error,
        };
    let _ = host_to;

    assert!(
        error
            .message
            .as_str()
            .contains("single_loop model requested tool 'apply_patch' that was not present")
    );
    assert_no_executed_calls(&host);
}

#[test]
fn single_loop_final_errors_when_model_calls_tool() {
    let input = workflow_input("change code");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![tool_call_response(call)])
        .with_tools(vec![read_file.clone()], vec![read_file]);
    let host_to = &mut host;

    let error = match run_single_loop(input, host_to, 0) {
        Ok(_) => panic!("workflow unexpectedly succeeded"),
        Err(error) => error,
    };
    let _ = host_to;

    assert!(
        error
            .message
            .as_str()
            .contains("single_loop_final model requested tool 'read_file' that was not present")
    );
    assert_no_executed_calls(&host);
}
