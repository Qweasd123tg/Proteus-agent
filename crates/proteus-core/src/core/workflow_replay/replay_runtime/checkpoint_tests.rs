use super::*;
use crate::{
    contracts::{
        WorkflowHistoryCheckpoint, WorkflowHistoryRecorder, WorkflowHistoryUpdate,
        WorkflowToolResultBinding,
    },
    domain::{ReasoningConfig, ToolCall},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};
use serde_json::json;

#[tokio::test]
async fn checkpoint_compares_execution_before_any_tool_is_requested() {
    for changed in [false, true] {
        let original = ToolCall::new("source-id", "original-tool", json!({"source": 1}));
        let execution = ToolCall::new("source-id", "target-tool", json!({"target": 2}));
        let user = CanonicalMessage::text(MessageRole::User, "prepare operation");
        let source_message = CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall {
                call: original.clone(),
            }],
        );
        let state = std::sync::Arc::new(ReplayState::new(
            vec![],
            vec![],
            vec![],
            None,
            Default::default(),
            &ReasoningConfig::default(),
        ));
        let recorder = ReplayCheckpointRecorder::new(
            state.clone(),
            vec![user.clone()],
            vec![RecordedCheckpoint {
                messages: vec![user, source_message],
                calls: vec![execution.clone()],
                position: (0, 0, 0),
            }],
        );
        // Replay ids may change, but the operation itself may not. There are
        // deliberately no model/tool exchanges to catch a changed declaration.
        let mut replay_original = original;
        replay_original.id = "replay-id".into();
        let mut replay_execution = execution;
        replay_execution.id = "replay-id".into();
        if changed {
            replay_execution.args = json!({"target": 3});
        }
        let checkpoint = WorkflowHistoryCheckpoint {
            history: WorkflowHistoryUpdate::new(vec![CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall {
                    call: replay_original,
                }],
            )]),
            tool_results: vec![WorkflowToolResultBinding::new(replay_execution)],
        };
        let result = recorder.checkpoint(checkpoint).await;
        if changed {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("checkpoint changed a tool execution binding")
            );
            assert!(!state.lock().issues.is_empty());
        } else {
            result.unwrap();
            recorder.finish();
            assert!(state.lock().issues.is_empty());
        }
    }
}

#[tokio::test]
async fn only_declared_in_flight_lifecycles_may_cross_a_checkpoint() {
    use super::super::RecordedToolInvocation;
    use crate::domain::{ToolCallResolution, ToolResult};

    for unrelated_crosses in [false, true] {
        // Identical operations deliberately have different outcomes. Their
        // checkpoint identities must survive reverse dispatch during replay.
        let source =
            ["source-a", "source-b", "unbound"].map(|id| ToolCall::new(id, "read", json!({})));
        let actual =
            ["replay-a", "replay-b", "unbound"].map(|id| ToolCall::new(id, "read", json!({})));
        let tools = source
            .iter()
            .map(|call| RecordedToolInvocation {
                call: call.clone(),
                approval_reason: None,
                resolution: ToolCallResolution::Allowed,
                result: ToolResult::ok(call.id.clone(), &call.id),
            })
            .collect();
        let state = Arc::new(ReplayState::new(
            vec![],
            tools,
            vec![],
            None,
            Default::default(),
            &ReasoningConfig::default(),
        ));
        let user = CanonicalMessage::text(MessageRole::User, "read concurrently");
        let message = |calls: &[ToolCall]| {
            CanonicalMessage::new(
                MessageRole::Assistant,
                calls
                    .iter()
                    .cloned()
                    .map(|call| ContentPart::ToolCall { call })
                    .collect(),
            )
        };
        let expected = RecordedCheckpoint {
            messages: vec![user.clone(), message(&source[..2])],
            calls: source[..2].to_vec(),
            position: (0, 0, 0),
        };
        let recorder = ReplayCheckpointRecorder::new(
            state.clone(),
            vec![user],
            vec![expected.clone(), expected],
        );
        let checkpoint = || WorkflowHistoryCheckpoint {
            history: WorkflowHistoryUpdate::new(vec![message(&actual[..2])]),
            tool_results: actual[..2]
                .iter()
                .cloned()
                .map(WorkflowToolResultBinding::new)
                .collect(),
        };
        recorder.checkpoint(checkpoint()).await.unwrap();
        for index in [1, 0] {
            state.record_tool_requested(&actual[index]).unwrap();
            state
                .record_tool_resolved(&actual[index], &ToolCallResolution::Allowed)
                .unwrap();
            let result = ToolResult::ok(actual[index].id.clone(), &source[index].id);
            state.record_tool_result(&result).unwrap();
        }
        if unrelated_crosses {
            state.record_tool_requested(&actual[2]).unwrap();
        }
        let result = recorder.checkpoint(checkpoint()).await;
        if unrelated_crosses {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("checkpoint moved across a model/tool boundary")
            );
        } else {
            result.unwrap();
            recorder.finish();
            assert!(state.lock().issues.is_empty());
            // Normalizing scheduling never excuses an omitted tool lifecycle.
            assert!(
                state
                    .summary()
                    .issues
                    .iter()
                    .any(|issue| issue.contains("unbound"))
            );
        }
    }
}
