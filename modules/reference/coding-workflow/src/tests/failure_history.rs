use super::*;

#[test]
fn regression_workflows_preserve_completed_tool_after_model_failure() {
    for deferred in [false, true] {
        let mut workflows: Vec<Box<dyn WorkflowModule>> = vec![
            Box::new(CodingSingleLoopWorkflow::default()),
            Box::new(CodingCodexLoopWorkflow),
        ];
        if !deferred {
            workflows.push(Box::new(CodingPlanExecuteReviewWorkflow));
        }
        for workflow in workflows {
            let call = ToolCall::new("completed-read", "read_file", json!({"path": "src/lib.rs"}));
            let model_call = if deferred {
                ToolCall::new(
                    call.id.clone(),
                    dynamic_tools::TOOL_CALL,
                    json!({"name": "read_file", "args": call.args}),
                )
            } else {
                call.clone()
            };
            let result = ToolResult::ok(call.id.clone(), "known file content");
            let read = test_tool("read_file", "Read file", ToolSafety::ReadOnly);
            let mut host = FakeHost::with_responses(vec![tool_call_response(model_call)])
                .with_tools(
                    vec![read.clone()],
                    if deferred { vec![] } else { vec![read] },
                )
                .with_tool_results(vec![result.clone()])
                .with_model_failure(2, ProcessModuleError::new("next request failed"));
            let failure = workflow
                .run_json(
                    serde_json::to_string(&workflow_input("read and explain")).unwrap(),
                    &mut host,
                )
                .unwrap_err();
            assert_eq!(host.executed_calls.lock().unwrap().as_slice(), &[call]);
            let history = failure
                .history
                .expect("completed effect needs failure history");
            assert!(history.new_messages.iter().any(|message| message.parts.iter().any(|part|
            matches!(&part.payload, ContentPart::ToolResult { result: actual } if actual == &result))));
            let checkpoints = host.checkpoints.lock().unwrap();
            assert!(
                checkpoints
                    .iter()
                    .any(|checkpoint| checkpoint.tool_results.len() == 1)
            );
            let captured = &checkpoints.last().unwrap().history;
            assert_eq!(captured.new_messages, history.new_messages);
            assert_eq!(captured.history_replacement, history.history_replacement);
        }
    }
}
