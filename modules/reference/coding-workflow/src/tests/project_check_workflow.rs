use super::*;

fn run_project_check(input: WorkflowModuleInput, host: &mut FakeHost) -> WorkflowModuleOutput {
    let input_json = serde_json::to_string(&input).expect("workflow input json");
    let output_json = CodingProjectCheckWorkflow
        .run_json(input_json, host)
        .unwrap_or_else(|error| panic!("project check failed: {}", error.message));
    serde_json::from_str(&output_json).expect("workflow output json")
}

fn successful_tool(output: &str, metadata: Value) -> ToolResult {
    ToolResult::ok(String::new(), output).with_metadata(metadata)
}

fn failed_test(output: &str, exit_code: i64) -> ToolResult {
    ToolResult::new(
        String::new(),
        false,
        output.to_owned(),
        Vec::new(),
        Some(format!("process exited with code {exit_code}")),
        json!({
            "exit_code": exit_code,
            "timed_out": false,
        }),
    )
}

#[test]
fn deterministic_project_check_passes_without_context_compaction_or_model() {
    let mut input = workflow_input("проверь проект");
    input.history.clear();
    let mut host = FakeHost::default().with_tool_results(vec![
        successful_tool("## main", json!({})),
        successful_tool("file\tCargo.toml\ndir\tsrc", json!({})),
        successful_tool(
            "test result: ok",
            json!({ "exit_code": 0, "timed_out": false }),
        ),
    ]);

    let output = run_project_check(input, &mut host);

    let calls = host.executed_calls.lock().expect("executed calls");
    assert_eq!(
        calls
            .iter()
            .map(|call| call.name.as_str())
            .collect::<Vec<_>>(),
        ["git_status", "list_dir", "shell"]
    );
    assert_eq!(calls[1].args, json!({ "path": "." }));
    assert_eq!(calls[2].args["command"], "cargo test");
    drop(calls);
    assert!(host.requests.lock().expect("requests").is_empty());
    assert!(host.compactions.lock().expect("compactions").is_empty());
    assert!(
        host.context_builds
            .lock()
            .expect("context builds")
            .is_empty()
    );
    assert_eq!(output.output.metadata["project_check"]["status"], "passed");
    assert_eq!(output.output.metadata["project_check"]["model_calls"], 0);
    assert_eq!(output.new_messages.len(), 1);
    assert!(output.output.text.contains("завершена успешно"));
}

#[test]
fn failed_tests_trigger_exactly_one_tool_free_model_explanation() {
    let response = CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "Ошибка находится в assertion."),
        Vec::new(),
        FinishReason::Stop,
    );
    let mut host = FakeHost::with_responses(vec![response]).with_tool_results(vec![
        successful_tool("## main\n M src/lib.rs", json!({})),
        successful_tool("file\tCargo.toml", json!({})),
        failed_test("assertion failed: left != right", 101),
    ]);

    let output = run_project_check(workflow_input("проверь проект"), &mut host);

    let requests = host.requests.lock().expect("requests");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].tools.is_empty());
    assert_eq!(requests[0].tool_choice, ToolChoice::None);
    assert_eq!(requests[0].messages.len(), 1);
    assert!(message_text(&requests[0].messages[0]).contains("assertion failed: left != right"));
    drop(requests);
    assert_eq!(output.output.metadata["project_check"]["status"], "failed");
    assert_eq!(output.output.metadata["project_check"]["model_calls"], 1);
    assert_eq!(
        output.output.metadata["project_check"]["project"]["kind"],
        "rust"
    );
    assert!(output.output.text.contains("Ошибка находится в assertion."));
}

#[test]
fn unsupported_project_stops_in_code_without_running_tests_or_model() {
    let mut host = FakeHost::default().with_tool_results(vec![
        successful_tool("## main", json!({})),
        successful_tool("file\tREADME.md", json!({})),
    ]);

    let output = run_project_check(workflow_input("проверь проект"), &mut host);

    let calls = host.executed_calls.lock().expect("executed calls");
    assert_eq!(
        calls
            .iter()
            .map(|call| call.name.as_str())
            .collect::<Vec<_>>(),
        ["git_status", "list_dir"]
    );
    drop(calls);
    assert!(host.requests.lock().expect("requests").is_empty());
    assert_eq!(
        output.output.metadata["project_check"]["status"],
        "unsupported"
    );
}

#[test]
fn policy_or_tool_failure_does_not_get_reinterpreted_by_the_model() {
    let denied = ToolResult::error(String::new(), "shell denied by policy");
    let mut host = FakeHost::default().with_tool_results(vec![
        successful_tool("## main", json!({})),
        successful_tool("file\tCargo.toml", json!({})),
        denied,
    ]);

    let output = run_project_check(workflow_input("проверь проект"), &mut host);

    assert!(host.requests.lock().expect("requests").is_empty());
    assert_eq!(output.output.metadata["project_check"]["status"], "blocked");
    assert!(output.output.text.contains("shell denied by policy"));
}
