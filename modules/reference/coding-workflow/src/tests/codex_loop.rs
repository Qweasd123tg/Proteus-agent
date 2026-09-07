use super::*;

#[test]
fn codex_loop_runs_tool_round_then_stops_on_non_tool_response() {
    let mut input = workflow_input("change code");
    input.runtime.instructions = vec![InstructionBlock::new(
        InstructionKind::System,
        "runtime codex base instructions",
        100,
    )];
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let apply_patch = test_tool("apply_patch", "Apply patch", ToolSafety::WritesFiles);
    let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![
        tool_call_response(call.clone()),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "final answer"),
            Vec::new(),
            FinishReason::Stop,
        ),
    ])
    .with_tools(vec![read_file.clone(), apply_patch], vec![read_file]);
    let host_to = &mut host;

    let output_json = match CodingCodexLoopWorkflow.run_json(String::from(input_json), host_to) {
        Ok(json) => json,
        Err(error) => panic!("workflow failed: {}", error.message),
    };
    let output: WorkflowModuleOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    let _ = host_to;

    assert_eq!(output.output.text, "final answer");
    assert_eq!(
        output.output.metadata["workflow"]["module_id"],
        CODEX_LOOP_MODULE_ID
    );
    assert_eq!(output.output.metadata["phases"], json!(["turn_loop"]));
    assert_eq!(output.output.metadata["tool_rounds"], json!(1));
    assert!(output.output.metadata["tool_round_limit_reached"].is_null());
    assert!(output.history_replacement.is_none());

    let persisted = output
        .new_messages
        .iter()
        .map(|message| (message.role.clone(), message_text(message)))
        .collect::<Vec<_>>();
    assert_eq!(
        persisted,
        vec![
            (MessageRole::Assistant, "<empty model response>".to_owned()),
            (MessageRole::Tool, "<empty model response>".to_owned()),
            (MessageRole::Assistant, "final answer".to_owned()),
        ]
    );
    let persisted_tool_output = output
        .new_messages
        .iter()
        .find_map(|message| {
            message.parts.iter().find_map(|part| match &part.payload {
                ContentPart::ToolResult { result } => Some(result.output.as_str()),
                _ => None,
            })
        })
        .expect("persisted tool result");
    assert_eq!(persisted_tool_output, "read_file ok");
    let executed_calls = host.executed_calls.lock().expect("executed calls");
    assert_eq!(executed_calls.len(), 1);
    assert_eq!(executed_calls[0].name, "read_file");

    let requests = host.requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0]
            .instructions
            .iter()
            .filter(|instruction| instruction.text == "runtime codex base instructions")
            .count(),
        1
    );
    assert!(
        !requests.iter().any(
            |request| request.instructions.iter().any(|instruction| instruction
                .text
                .contains("Codex execute phase")
                || instruction.text.contains("Codex final phase")
                || instruction.text.contains("Codex-shaped coding workflow"))
        )
    );
    assert!(
        requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == dynamic_tools::TOOL_CALL)
    );
    assert!(requests[1].messages.iter().any(|message| {
        message.parts.iter().any(|part| match &part.payload {
            ContentPart::ToolResult { result } => result.output == "read_file ok",
            _ => false,
        })
    }));
    assert!(
        requests[1]
            .tools
            .iter()
            .any(|tool| tool.name == dynamic_tools::TOOL_CALL)
    );

    let compactions = host.compactions.lock().expect("compactions");
    assert_eq!(compactions.len(), 2);
    assert_eq!(compactions[0].reason.as_deref(), Some("codex_loop"));
    assert_eq!(compactions[1].reason.as_deref(), Some("codex_loop"));
}

#[test]
fn codex_loop_continues_when_provider_sets_end_turn_false() {
    let input = workflow_input("continue until the provider ends the turn");
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut host = FakeHost::with_responses(vec![
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "intermediate"),
            Vec::new(),
            FinishReason::Stop,
        )
        .with_end_turn(false),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "final answer"),
            Vec::new(),
            FinishReason::Stop,
        )
        .with_end_turn(true),
    ]);
    let host_to = &mut host;

    let output_json = match CodingCodexLoopWorkflow.run_json(String::from(input_json), host_to) {
        Ok(json) => json,
        Err(error) => panic!("workflow failed: {}", error.message),
    };
    let output: WorkflowModuleOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    let _ = host_to;

    assert_eq!(output.output.text, "final answer");
    assert_eq!(host.requests.lock().expect("requests").len(), 2);
    let assistant_texts = output
        .new_messages
        .iter()
        .filter(|message| message.role == MessageRole::Assistant)
        .map(message_text)
        .collect::<Vec<_>>();
    assert_eq!(assistant_texts, ["intermediate", "final answer"]);
    assert_no_executed_calls(&host);
}

#[test]
fn codex_loop_preserves_commentary_and_uses_the_last_message_as_final_output() {
    let input = workflow_input("preserve Codex response item phases");
    let input_json = serde_json::to_string(&input).expect("input json");
    let response = CanonicalModelResponse::from_messages(
        vec![
            CanonicalMessage::text(MessageRole::Assistant, "checking files")
                .with_phase(MessagePhase::Commentary),
            CanonicalMessage::text(MessageRole::Assistant, "done")
                .with_phase(MessagePhase::FinalAnswer),
        ],
        Vec::new(),
        FinishReason::Stop,
    );
    let mut host = FakeHost::with_responses(vec![response]);

    let output_json = CodingCodexLoopWorkflow
        .run_json(input_json, &mut host)
        .unwrap_or_else(|error| panic!("workflow failed: {}", error.message));
    let output: WorkflowModuleOutput =
        serde_json::from_str(&output_json).expect("workflow output json");

    assert_eq!(output.output.text, "done");
    let assistant_messages = output
        .new_messages
        .iter()
        .filter(|message| message.role == MessageRole::Assistant)
        .collect::<Vec<_>>();
    assert_eq!(assistant_messages.len(), 2);
    assert_eq!(assistant_messages[0].phase, Some(MessagePhase::Commentary));
    assert_eq!(assistant_messages[1].phase, Some(MessagePhase::FinalAnswer));
}

#[test]
fn codex_loop_empty_final_response_stays_strict_by_default() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![
        tool_call_response(call),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, ""),
            Vec::new(),
            FinishReason::Stop,
        ),
    ])
    .with_tools(vec![read_file.clone()], vec![read_file]);
    let host_to = &mut host;

    let output_json = match CodingCodexLoopWorkflow.run_json(String::from(input_json), host_to) {
        Ok(json) => json,
        Err(error) => panic!("workflow failed: {}", error.message),
    };
    let output: WorkflowModuleOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");

    assert_eq!(output.output.text, "<empty model response>");
    assert!(!output.output.text.contains("read_file ok"));
    assert!(
        !output.new_messages.is_empty(),
        "even an empty final response must persist the completed turn"
    );
    assert_eq!(
        output.new_messages.last().map(|message| &message.role),
        Some(&MessageRole::Assistant)
    );
}

#[test]
fn codex_loop_errors_on_tool_finish_without_tool_calls() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut host = FakeHost::with_responses(vec![CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, ""),
        Vec::new(),
        FinishReason::ToolCalls,
    )]);
    let host_to = &mut host;

    let error = match CodingCodexLoopWorkflow.run_json(String::from(input_json), host_to) {
        Ok(_) => panic!("workflow unexpectedly succeeded"),
        Err(error) => error,
    };
    let _ = host_to;

    assert!(
        error
            .message
            .as_str()
            .contains("ToolCalls without tool calls")
    );
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
    assert!(
        host.events
            .lock()
            .expect("events")
            .iter()
            .all(|event| !matches!(event, Event::TurnFinished { .. }))
    );
}

#[test]
fn codex_loop_errors_on_length_response() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut host = FakeHost::with_responses(vec![CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "partial"),
        Vec::new(),
        FinishReason::Length,
    )]);
    let host_to = &mut host;

    let error = match CodingCodexLoopWorkflow.run_json(String::from(input_json), host_to) {
        Ok(_) => panic!("workflow unexpectedly succeeded"),
        Err(error) => error,
    };
    let _ = host_to;

    assert!(error.message.as_str().contains("length limit"));
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}

#[test]
fn codex_loop_errors_when_tool_calls_do_not_match_message_parts() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let read_file = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let call = ToolCall::new(new_call_id(), "read_file", json!({ "path": "src/lib.rs" }));
    let mut host = FakeHost::with_responses(vec![CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "calling tool"),
        vec![call],
        FinishReason::ToolCalls,
    )])
    .with_tools(vec![read_file.clone()], vec![read_file]);
    let host_to = &mut host;

    let error = match CodingCodexLoopWorkflow.run_json(String::from(input_json), host_to) {
        Ok(_) => panic!("workflow unexpectedly succeeded"),
        Err(error) => error,
    };
    let _ = host_to;

    assert!(
        error
            .message
            .as_str()
            .contains("does not match assistant message")
    );
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}

#[test]
fn codex_loop_rejects_message_only_tool_call_before_finishing() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let hidden_call = ToolCall::new(
        new_call_id(),
        "hidden_write",
        json!({ "path": "outside.txt" }),
    );
    let mut host = FakeHost::with_responses(vec![CanonicalModelResponse::new(
        CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall { call: hidden_call }],
        ),
        Vec::new(),
        FinishReason::Stop,
    )]);
    let host_to = &mut host;

    let error = match CodingCodexLoopWorkflow.run_json(String::from(input_json), host_to) {
        Ok(_) => panic!("workflow unexpectedly succeeded"),
        Err(error) => error,
    };
    let _ = host_to;

    assert!(
        error
            .message
            .as_str()
            .contains("does not match assistant message")
    );
    assert_no_executed_calls(&host);
}

#[test]
fn codex_loop_returns_unrequested_tool_error_to_model_without_execution() {
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
        tool_call_response(call.clone()),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "recovered final"),
            Vec::new(),
            FinishReason::Stop,
        ),
    ])
    .with_tools(vec![read_file.clone(), apply_patch], vec![read_file]);
    let host_to = &mut host;

    let output_json = match CodingCodexLoopWorkflow.run_json(String::from(input_json), host_to) {
        Ok(output) => output,
        Err(error) => panic!("workflow failed: {}", error.message),
    };
    let output: WorkflowModuleOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    let _ = host_to;

    assert_eq!(output.output.text, "recovered final");
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
    let requests = host.requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    let returned_error = requests[1]
        .messages
        .iter()
        .flat_map(|message| &message.parts)
        .find_map(|part| match &part.payload {
            ContentPart::ToolResult { result } if result.call_id == call.id => Some(result),
            _ => None,
        })
        .expect("unsupported tool result in follow-up request");
    assert!(!returned_error.ok);
    assert_eq!(
        returned_error.error.as_deref(),
        Some("unsupported call: apply_patch")
    );
}

#[test]
fn codex_loop_errors_when_changed_compaction_drops_current_user_message() {
    let input = workflow_input("change code");
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut bad_output = proteus_contracts::contracts::CompactionOutput::changed(
        vec![CanonicalMessage::text(MessageRole::User, "summary only")],
        Some("summary only".to_owned()),
    );
    bad_output.original_token_estimate = Some(100);
    bad_output.token_estimate = Some(10);
    let mut host = FakeHost::default().with_compaction_outputs(vec![bad_output]);
    let host_to = &mut host;

    let error = match CodingCodexLoopWorkflow.run_json(String::from(input_json), host_to) {
        Ok(_) => panic!("workflow unexpectedly succeeded"),
        Err(error) => error,
    };
    let _ = host_to;

    assert!(
        error
            .message
            .as_str()
            .contains("dropped the current user message")
    );
    assert!(host.requests.lock().expect("requests").is_empty());
    assert!(
        host.events
            .lock()
            .expect("events")
            .iter()
            .all(|event| !matches!(event, Event::TurnFinished { .. }))
    );
}

#[test]
fn codex_loop_separates_compacted_history_from_new_turn_messages() {
    let input = workflow_input("change code");
    let current_user = input.history.last().expect("current user").clone();
    let generated_summary = CanonicalMessage::text(MessageRole::User, "compacted summary")
        .with_metadata(json!({ "generated": true, "summary": true }));
    let compacted_history = vec![current_user.clone(), generated_summary.clone()];
    let compacted_output = proteus_contracts::contracts::CompactionOutput::changed(
        compacted_history.clone(),
        Some("compacted summary".to_owned()),
    );
    let input_json = serde_json::to_string(&input).expect("input json");
    let mut host = FakeHost::default().with_compaction_outputs(vec![compacted_output]);
    let host_to = &mut host;

    let output_json = match CodingCodexLoopWorkflow.run_json(String::from(input_json), host_to) {
        Ok(output) => output,
        Err(error) => panic!("workflow failed: {}", error.message),
    };
    let output: WorkflowModuleOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    let _ = host_to;

    assert_eq!(output.history_replacement, Some(compacted_history));
    assert_eq!(output.new_messages.len(), 1);
    assert_eq!(output.new_messages[0].role, MessageRole::Assistant);
    assert_eq!(message_text(&output.new_messages[0]), "done");
}
