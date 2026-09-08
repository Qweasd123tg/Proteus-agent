use super::*;

fn shell_call() -> ToolCall {
    ToolCall::new(
        "patch-call",
        "shell",
        json!({"command": "apply_patch <<'PATCH'\n*** Begin Patch\n*** Add File: proof\n+ok\n*** End Patch\nPATCH"}),
    )
}

fn host_for(call: ToolCall, selected: Vec<ToolSpec>) -> FakeHost {
    FakeHost::with_responses(vec![tool_call_response(call)]).with_tools(
        vec![
            test_tool("shell", "Shell", ToolSafety::RunsCommands),
            test_tool("apply_patch", "Patch", ToolSafety::WritesFiles),
        ],
        selected,
    )
}

#[test]
fn codex_keeps_model_call_and_declares_the_adapted_execution_once() {
    let call = shell_call();
    // The target is not model-visible, but the host must still decide its
    // policy/safety outcome; only the ORIGINAL call requires model visibility.
    let mut host = host_for(
        call.clone(),
        vec![test_tool("shell", "Shell", ToolSafety::RunsCommands)],
    );
    CodingCodexLoopWorkflow
        .run_json(
            serde_json::to_string(&workflow_input("patch")).unwrap(),
            &mut host,
        )
        .unwrap();
    let executed = host.executed_calls.lock().unwrap();
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].name, "apply_patch");
    assert_eq!(executed[0].id, call.id);
    let checkpoints = host.checkpoints.lock().unwrap();
    assert_eq!(checkpoints[0].tool_results[0].execution_call, executed[0]);
    let requests = host.requests.lock().unwrap();
    let original = requests[1]
        .messages
        .iter()
        .flat_map(|m| &m.parts)
        .find_map(|p| match &p.payload {
            ContentPart::ToolCall { call } => Some(call),
            _ => None,
        })
        .unwrap();
    assert_eq!(original, &call);
}

#[test]
fn hidden_shell_is_not_adapted_into_a_visible_patch_tool() {
    let call = shell_call();
    let mut host = host_for(
        call,
        vec![test_tool("apply_patch", "Patch", ToolSafety::WritesFiles)],
    );
    CodingCodexLoopWorkflow
        .run_json(
            serde_json::to_string(&workflow_input("patch")).unwrap(),
            &mut host,
        )
        .unwrap();
    assert!(host.executed_calls.lock().unwrap().is_empty());
    assert!(host.checkpoints.lock().unwrap()[0].tool_results.is_empty());
    assert!(host.requests.lock().unwrap()[1].messages.iter().flat_map(|m| &m.parts).any(|p| {
        matches!(&p.payload, ContentPart::ToolResult { result } if result.error.as_deref() == Some("unsupported call: shell"))
    }));
}

#[test]
fn ordinary_workflow_does_not_apply_codex_command_adaptation() {
    let call = shell_call();
    let mut host = host_for(
        call.clone(),
        vec![
            test_tool("shell", "Shell", ToolSafety::RunsCommands),
            test_tool("apply_patch", "Patch", ToolSafety::WritesFiles),
        ],
    );
    CodingSingleLoopWorkflow::default()
        .run_json(
            serde_json::to_string(&workflow_input("patch")).unwrap(),
            &mut host,
        )
        .unwrap();
    assert_eq!(*host.executed_calls.lock().unwrap(), vec![call]);
}

#[test]
fn invalid_raw_shell_arguments_are_not_repaired_by_patch_adaptation() {
    let call = shell_call().with_raw_arguments("{ invalid");
    let mut host = host_for(
        call.clone(),
        vec![test_tool("shell", "Shell", ToolSafety::RunsCommands)],
    );
    CodingCodexLoopWorkflow
        .run_json(
            serde_json::to_string(&workflow_input("patch")).unwrap(),
            &mut host,
        )
        .unwrap();
    assert_eq!(*host.executed_calls.lock().unwrap(), vec![call]);
}
