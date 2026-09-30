use super::super::super::fixture::RecordedToolInvocation;
use super::*;
use crate::{
    contracts::{ExecutionAttribution, HookStep, HookTrace},
    domain::{
        AgentTask, ReasoningConfig, ToolCall, ToolCallResolution, ToolResult, new_execution_id,
        new_message_id,
    },
};

fn input(event: HookEvent) -> HookInput {
    HookInput {
        event,
        attribution: ExecutionAttribution::detached(new_execution_id()),
        cwd: ".".into(),
    }
}
fn state(hooks: Vec<HookTrace>, tools: Vec<RecordedToolInvocation>) -> Arc<ReplayState> {
    Arc::new(ReplayState::new(
        vec![],
        hooks,
        tools,
        vec![],
        None,
        Default::default(),
        &ReasoningConfig::default(),
        new_message_id(),
    ))
}

#[tokio::test]
async fn tool_replay_uses_raw_outcome_then_applies_recorded_output_once() {
    let call = ToolCall::new("recorded".to_owned(), "read_file", serde_json::json!({}));
    let raw = ToolResult::ok(call.id.clone(), "raw");
    let before = input(HookEvent::AfterTool {
        call: call.clone(),
        result: raw.clone(),
    });
    let response = HookResponse::ToolOutput {
        output: "shaped".into(),
    };
    let output = apply_hook_response(&before.event, &response).unwrap();
    let state = state(
        vec![HookTrace {
            input: before,
            steps: vec![HookStep {
                module_id: "budget".into(),
                outcome: HookStepOutcome::Accepted { response },
            }],
            output: Some(output),
        }],
        vec![RecordedToolInvocation {
            call: call.clone(),
            approval_reason: None,
            resolution: ToolCallResolution::Allowed,
            result: ToolResult::ok(call.id.clone(), "shaped"),
            raw_result: Some(raw),
        }],
    );
    let mut actual = call;
    actual.id = "actual".into();
    state.record_tool_requested(&actual).unwrap();
    let result = state.replay_tool_result(&actual.id).unwrap();
    assert_eq!(result.output, "raw");
    assert_eq!(result.call_id, "actual");
    let replay = ReplayHooks::new(state.clone(), vec!["budget".into()]);
    let output = replay
        .apply(input(HookEvent::AfterTool {
            call: actual.clone(),
            result,
        }))
        .await
        .unwrap();
    let HookEvent::AfterTool { result, .. } = output else {
        panic!("tool output")
    };
    assert_eq!(result.output, "shaped");
    assert_eq!(result.call_id, "actual");
    state
        .record_tool_resolved(&actual, &ToolCallResolution::Allowed)
        .unwrap();
    state.record_tool_result(&result).unwrap();
    assert!(state.summary().issues.is_empty());
}

#[tokio::test]
async fn recorded_failure_is_returned_without_executing_any_handler() {
    let call = ToolCall::new("recorded".to_owned(), "read_file", serde_json::json!({}));
    let before = input(HookEvent::BeforeTool {
        call,
        spec: None,
        blocked: None,
    });
    let state = state(
        vec![HookTrace {
            input: before.clone(),
            steps: vec![HookStep {
                module_id: "guard".into(),
                outcome: HookStepOutcome::Failed {
                    message: "recorded handler failure".into(),
                },
            }],
            output: None,
        }],
        vec![],
    );
    let replay = ReplayHooks::new(state.clone(), vec!["guard".into()]);
    assert_eq!(
        replay.apply(before).await.unwrap_err().to_string(),
        "recorded handler failure"
    );
    assert!(state.summary().issues.is_empty());
}

#[tokio::test]
async fn notification_failure_is_best_effort_and_missing_boundaries_are_reported() {
    let before = input(HookEvent::TurnStarted {
        task: AgentTask::new("task", ".".into()),
        history: vec![],
    });
    let trace = HookTrace {
        input: before.clone(),
        steps: vec![HookStep {
            module_id: "observer".into(),
            outcome: HookStepOutcome::Failed {
                message: "unavailable".into(),
            },
        }],
        output: Some(before.event.clone()),
    };
    let state = state(vec![trace], vec![]);
    let replay = ReplayHooks::new(state.clone(), vec!["observer".into()]);
    assert_eq!(replay.apply(before.clone()).await.unwrap(), before.event);
    assert!(state.summary().issues.is_empty());
    assert!(replay.apply(before).await.is_err());
    assert!(!state.summary().issues.is_empty());
}

#[tokio::test]
async fn recorded_order_must_match_the_snapshot_even_without_workers() {
    let before = input(HookEvent::TurnStarted {
        task: AgentTask::new("task", ".".into()),
        history: vec![],
    });
    let trace = HookTrace {
        input: before.clone(),
        steps: vec![HookStep {
            module_id: "different".into(),
            outcome: HookStepOutcome::Accepted {
                response: HookResponse::Continue,
            },
        }],
        output: Some(before.event.clone()),
    };
    let state = state(vec![trace], vec![]);
    let replay = ReplayHooks::new(state.clone(), vec!["configured".into()]);
    assert!(
        replay
            .apply(before)
            .await
            .unwrap_err()
            .to_string()
            .contains("order differs")
    );
    assert!(!state.summary().issues.is_empty());
}

#[test]
fn pending_pre_model_request_is_available_without_a_recorded_exchange() {
    let request = crate::model_standard::CanonicalModelRequest::new(
        crate::domain::ModelRef::new("fake", "fake"),
        vec![],
    );
    let before = input(HookEvent::BeforeModel {
        origin: crate::contracts::ModelCallOrigin::Direct,
        request: request.clone(),
    });
    let state = state(
        vec![HookTrace {
            input: before,
            steps: vec![HookStep {
                module_id: "guard".into(),
                outcome: HookStepOutcome::Failed {
                    message: "stop".into(),
                },
            }],
            output: None,
        }],
        vec![],
    );
    assert_eq!(state.current_request().unwrap(), request);
}
