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
