use super::*;

#[test]
fn proteus_tool_describe_returns_policy_visible_hidden_schema() {
    let input = workflow_input("describe hidden tool");
    let git_log = test_tool("git_log", "Show commit history", ToolSafety::ReadOnly);
    let mut host = FakeHost::default().with_tools(vec![git_log], Vec::new());
    let host_to = &mut host;
    let call = ToolCall::new(
        new_call_id(),
        dynamic_tools::TOOL_DESCRIBE,
        json!({ "name": "git_log" }),
    );

    let result = dynamic_tools::handle_meta_tool_call(host_to, &input, &call, "execute").unwrap();
    let _ = host_to;
    let output: Value = serde_json::from_str(&result.output).expect("describe output json");

    assert!(result.ok);
    assert_eq!(result.call_id, call.id);
    assert_eq!(output["name"], "git_log");
    assert_eq!(output["required_args"], Value::Null);
    assert_eq!(output["input_schema"]["required"], json!(["path"]));
}

#[test]
fn proteus_tool_search_returns_compact_policy_visible_matches() {
    let input = workflow_input("search hidden tools");
    let git_log = test_tool("git_log", "Show commit history", ToolSafety::ReadOnly);
    let shell = test_tool("shell", "Run terminal commands", ToolSafety::RunsCommands);
    let mut host = FakeHost::default().with_tools(vec![git_log, shell], Vec::new());
    let host_to = &mut host;
    let call = ToolCall::new(
        new_call_id(),
        dynamic_tools::TOOL_SEARCH,
        json!({ "query": "commit history", "limit": 3 }),
    );

    let result = dynamic_tools::handle_meta_tool_call(host_to, &input, &call, "execute").unwrap();
    let _ = host_to;
    let output: Value = serde_json::from_str(&result.output).expect("search output json");

    assert!(result.ok);
    assert_eq!(result.call_id, call.id);
    assert_eq!(output["matches"][0]["name"], "git_log");
    assert_eq!(output["matches"][0]["input_schema"], Value::Null);
    assert_eq!(output["matches"][0]["required_args"], json!(["path"]));
}

#[test]
fn proteus_tool_call_executes_hidden_tool_and_remaps_result_to_outer_call_id() {
    let outer_call = ToolCall::new(
        new_call_id(),
        dynamic_tools::TOOL_CALL,
        json!({
            "name": "hidden_echo",
            "args": { "path": "README.md" }
        }),
    );
    let input = workflow_input("call hidden tool");
    let input_json = serde_json::to_string(&input).expect("input json");
    let hidden_echo = test_tool("hidden_echo", "Echo hidden file", ToolSafety::ReadOnly);
    let mut host = FakeHost::with_responses(vec![
        tool_call_response(outer_call.clone()),
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "final"),
            Vec::new(),
            FinishReason::Stop,
        ),
    ])
    .with_tools(vec![hidden_echo], Vec::new());
    let host_to = &mut host;

    let output_json =
        match CodingSingleLoopWorkflow::default().run_json(String::from(input_json), host_to) {
            Ok(json) => json,
            Err(error) => panic!("workflow failed: {}", error.message),
        };
    let output: WorkflowModuleOutput =
        serde_json::from_str(output_json.as_str()).expect("output json");
    let _ = host_to;

    let executed_calls = host.executed_calls.lock().expect("executed calls");
    assert_eq!(executed_calls.len(), 1);
    assert_eq!(executed_calls[0].name, "hidden_echo");
    assert_ne!(executed_calls[0].id, outer_call.id);

    let result = output
        .new_messages
        .iter()
        .find_map(|message| {
            message.parts.iter().find_map(|part| match &part.payload {
                ContentPart::ToolResult { result } => Some(result),
                _ => None,
            })
        })
        .expect("tool result");
    assert_eq!(result.call_id, outer_call.id);
    assert_eq!(
        result.metadata["deferred_tool"]["name"],
        Value::String("hidden_echo".to_owned())
    );
    assert_eq!(
        result.metadata["deferred_tool"]["inner_call_id"],
        Value::String(executed_calls[0].id.clone())
    );
}

#[test]
fn proteus_tool_call_rejects_meta_tool_recursion_without_execution() {
    let input = workflow_input("bad recursive call");
    let mut host = FakeHost::default();
    let host_to = &mut host;
    let call = ToolCall::new(
        new_call_id(),
        dynamic_tools::TOOL_CALL,
        json!({
            "name": dynamic_tools::TOOL_SEARCH,
            "args": { "query": "anything" }
        }),
    );

    let result = dynamic_tools::handle_meta_tool_call(host_to, &input, &call, "execute").unwrap();
    let _ = host_to;

    assert!(!result.ok);
    assert_eq!(result.call_id, call.id);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("cannot call Proteus meta-tools")
    );
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}

#[test]
fn proteus_tool_call_rejects_provider_hosted_tool_without_execution() {
    let input = workflow_input("bad hosted call");
    let web_search = test_tool("web_search", "Search the web", ToolSafety::Network).with_surface(
        ToolSurface::provider_hosted(HostedToolConfig::WebSearch {
            config: WebSearchHostedToolConfig::default(),
        }),
    );
    let mut host = FakeHost::default().with_tools(vec![web_search], Vec::new());
    let host_to = &mut host;
    let call = ToolCall::new(
        new_call_id(),
        dynamic_tools::TOOL_CALL,
        json!({ "name": "web_search", "args": {} }),
    );

    let result = dynamic_tools::handle_meta_tool_call(host_to, &input, &call, "execute").unwrap();
    let _ = host_to;

    assert!(!result.ok);
    assert_eq!(result.call_id, call.id);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("provider-hosted tool 'web_search' cannot be invoked")
    );
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}

#[test]
fn proteus_tool_call_rejects_non_readonly_hidden_tool_in_plan_phase() {
    let input = workflow_input("plan write");
    let write_file = test_tool("write_file", "Write a file", ToolSafety::WritesFiles);
    let mut host = FakeHost::default().with_tools(vec![write_file], Vec::new());
    let host_to = &mut host;
    let call = ToolCall::new(
        new_call_id(),
        dynamic_tools::TOOL_CALL,
        json!({
            "name": "write_file",
            "args": { "path": "README.md" }
        }),
    );

    let result = dynamic_tools::handle_meta_tool_call(host_to, &input, &call, "plan").unwrap();
    let _ = host_to;

    assert!(!result.ok);
    assert_eq!(result.call_id, call.id);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("plan phase")
    );
    assert!(
        host.executed_calls
            .lock()
            .expect("executed calls")
            .is_empty()
    );
}
