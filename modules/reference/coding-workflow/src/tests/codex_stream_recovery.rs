use super::*;
use proteus_contracts::model_standard::{ModelFailure, ModelFailureKind};

#[test]
fn codex_stream_retry_budget_is_per_sampling_request_and_preserves_progress() {
    let mut input = workflow_input("read and explain");
    input.config = json!({"stream_max_retries": 1});
    let call = ToolCall::new(new_call_id(), "read_file", json!({"path": "src/lib.rs"}));
    let read = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
    let first = CanonicalMessage::text(MessageRole::Assistant, "first progress")
        .with_phase(MessagePhase::Commentary);
    let second = CanonicalMessage::text(MessageRole::Assistant, "second progress")
        .with_phase(MessagePhase::Commentary);
    let mut host = FakeHost::with_responses(vec![tool_call_response(call.clone())])
        .with_tools(vec![read.clone()], vec![read]);
    *host.model_failures.lock().unwrap() = VecDeque::from([
        (1, disconnected(first.clone())),
        (3, disconnected(second.clone())),
    ]);

    let output: WorkflowModuleOutput = serde_json::from_str(
        &CodingCodexLoopWorkflow
            .run_json(serde_json::to_string(&input).unwrap(), &mut host)
            .expect("each successful sampling request resets the stream budget"),
    )
    .unwrap();
    let requests = host.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[1].messages.last(), Some(&first));
    assert_eq!(requests[3].messages.last(), Some(&second));
    assert_eq!(host.executed_calls.lock().unwrap().as_slice(), &[call]);
    assert_eq!(
        host.compactions.lock().unwrap().len(),
        2,
        "no extra compaction phase on reconnect"
    );
    for progress in [&first, &second] {
        assert_eq!(
            output
                .new_messages
                .iter()
                .filter(|m| *m == progress)
                .count(),
            1
        );
        assert_eq!(
            requests[3]
                .messages
                .iter()
                .filter(|m| *m == progress)
                .count(),
            1
        );
        assert!(
            host.checkpoints
                .lock()
                .unwrap()
                .iter()
                .any(|checkpoint| checkpoint.history.new_messages.contains(progress))
        );
    }
}

#[test]
fn codex_stream_completed_items_do_not_reset_the_retry_budget() {
    let mut input = workflow_input("explain");
    input.config = json!({"stream_max_retries": 1});
    let first = CanonicalMessage::text(MessageRole::Assistant, "first");
    let second = CanonicalMessage::text(MessageRole::Assistant, "second");
    let terminal = disconnected(second.clone());
    let mut host = FakeHost::default();
    *host.model_failures.lock().unwrap() =
        VecDeque::from([(1, disconnected(first.clone())), (2, terminal.clone())]);
    let failure = CodingCodexLoopWorkflow
        .run_json(serde_json::to_string(&input).unwrap(), &mut host)
        .expect_err("completed messages must not grant another retry");
    assert_eq!(host.requests.lock().unwrap().len(), 2);
    assert_eq!(failure.model_failure, terminal.model_failure);
    assert_eq!(failure.history.unwrap().new_messages, vec![first, second]);
}

#[test]
fn codex_stream_retry_requires_the_typed_cause_and_can_be_disabled() {
    for (kind, retries) in [
        (ModelFailureKind::StreamDisconnected, 0),
        (ModelFailureKind::Other, 5),
        (ModelFailureKind::Interrupted, 5),
        (ModelFailureKind::SessionBudgetExceeded, 5),
        (ModelFailureKind::ContextWindowExceeded, 5),
    ] {
        let mut input = workflow_input("explain");
        input.config = json!({"stream_max_retries": retries});
        let expected = ModelFailure::new(kind, "sse transport error: same text");
        let mut host = FakeHost::default()
            .with_model_failure(1, ProcessModuleError::from_model_failure(expected.clone()));
        let failure = CodingCodexLoopWorkflow
            .run_json(serde_json::to_string(&input).unwrap(), &mut host)
            .expect_err("no retry");
        assert_eq!(host.requests.lock().unwrap().len(), 1);
        assert_eq!(failure.model_failure, Some(expected));
    }
}

fn disconnected(message: CanonicalMessage) -> ProcessModuleError {
    ProcessModuleError::from_model_failure(
        ModelFailure::new(ModelFailureKind::StreamDisconnected, "stream disconnected")
            .with_completed_messages(vec![message]),
    )
}

#[test]
fn completed_tools_are_drained_before_terminal_model_failure() {
    for kind in [
        ModelFailureKind::StreamDisconnected,
        ModelFailureKind::Other,
    ] {
        let mut input = workflow_input("read then handle failure");
        input.config = json!({"stream_max_retries": 0});
        let call = ToolCall::new("completed_call", "read_file", json!({"path": "src/lib.rs"}));
        let message = tool_call_response(call.clone()).messages.remove(0);
        let read = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
        let expected = ModelFailure::new(kind, "terminal sample error")
            .with_completed_messages(vec![message.clone()]);
        let mut host = FakeHost::default()
            .with_tools(vec![read.clone()], vec![read])
            .with_model_failure(1, ProcessModuleError::from_model_failure(expected.clone()));
        let failure = CodingCodexLoopWorkflow
            .run_json(serde_json::to_string(&input).unwrap(), &mut host)
            .unwrap_err();
        assert_eq!(failure.model_failure, Some(expected));
        assert_eq!(
            host.executed_calls.lock().unwrap().as_slice(),
            &[call.clone()]
        );
        let history = failure.history.unwrap().new_messages;
        assert_eq!(history[0], message);
        assert!(
            matches!(&history[1].parts[0].payload, ContentPart::ToolResult { result } if result.call_id == call.id)
        );
        assert_eq!(
            host.checkpoints.lock().unwrap()[0].tool_results[0].execution_call,
            call
        );
        assert_eq!(host.requests.lock().unwrap().len(), 1);
    }
}
