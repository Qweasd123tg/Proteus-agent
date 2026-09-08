use super::*;
use crate::{
    contracts::ExecutionAttribution,
    contracts::ToolExecutionRecorder,
    core::{SessionToolExecutionRecorder, TurnOpened},
    domain::{
        AgentTask, ToolCall, ToolCallResolution, ToolResult, new_execution_id, new_session_id,
        new_thread_id, new_turn_id,
    },
};
use serde_json::json;

#[tokio::test]
async fn checkpoint_captures_only_declared_root_results_in_binding_order() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let store = SessionStore::new(root.path(), &workspace, new_session_id()).unwrap();
    let thread = new_thread_id();
    let turn = new_turn_id();
    let attribution =
        ExecutionAttribution::for_turn(new_execution_id(), store.session_id(), thread, turn);
    store
        .append_execution_journal_entry(
            attribution,
            JournalEntry::TurnOpened(TurnOpened {
                task: AgentTask::new("capture", workspace),
                base_history_revision: 0,
                module_epoch: 0,
                config_snapshot: json!({}),
            }),
        )
        .await
        .unwrap();
    let user = CanonicalMessage::text(MessageRole::User, "capture");
    store
        .append_history(thread, Some(turn), std::slice::from_ref(&user))
        .await
        .unwrap();
    let calls = [
        ToolCall::new("first", "read_file", json!({"path": "a"})),
        ToolCall::new("second", "read_file", json!({"path": "b"})),
    ];
    let assistant = CanonicalMessage::new(
        MessageRole::Assistant,
        calls
            .iter()
            .cloned()
            .map(|call| ContentPart::ToolCall { call })
            .collect(),
    );
    let history = vec![user, assistant];
    let bindings = calls
        .iter()
        .map(|call| WorkflowToolResultBinding::new(call.id.clone()))
        .collect::<Vec<_>>();

    // Reject malformed declarations before any result can be adopted.
    for invalid in [
        vec![WorkflowToolResultBinding::new("absent".into())],
        vec![bindings[0].clone(), bindings[0].clone()],
    ] {
        assert!(
            store
                .checkpoint_history(thread, turn, &history, None, invalid)
                .await
                .is_err()
        );
    }
    assert_eq!(store.load_projection().unwrap().history_revision, 1);
    store
        .checkpoint_history(thread, turn, &history, None, bindings.clone())
        .await
        .unwrap();
    let recorder = SessionToolExecutionRecorder::new(store.clone());
    let mut forged_call = calls[0].clone();
    forged_call.args = json!({"path": "changed"});
    assert!(
        recorder
            .tool_call_requested(attribution, &forged_call)
            .await
            .is_err()
    );

    let undeclared = ToolCall::new("other", "read_file", json!({"path": "c"}));
    for call in calls.iter().chain(std::iter::once(&undeclared)) {
        recorder
            .tool_call_requested(attribution, call)
            .await
            .unwrap();
        recorder
            .tool_call_resolved(attribution, call, &ToolCallResolution::Allowed)
            .await
            .unwrap();
    }
    recorder
        .tool_result_recorded(
            attribution,
            &ToolResult::ok(undeclared.id, "not conversation history"),
        )
        .await
        .unwrap();
    let second = ToolResult::ok(calls[1].id.clone(), "second completed first");
    recorder
        .tool_result_recorded(attribution, &second)
        .await
        .unwrap();
    let cold = SessionStore::open(store.session_dir().to_owned())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(cold.history_revision, 3);
    assert_eq!(
        cold.history.last(),
        Some(&bindings[1].message(second.clone()))
    );
    assert_eq!(cold.unresolved_tool_calls, vec![calls[0].id.clone()]);
    let first = ToolResult::ok(calls[0].id.clone(), "first completed later");
    recorder
        .tool_result_recorded(attribution, &first)
        .await
        .unwrap();
    let cold = store.load_projection().unwrap();
    assert_eq!(cold.history_revision, 4);
    assert_eq!(&cold.history[..2], history.as_slice());
    assert_eq!(
        &cold.history[2..],
        &[bindings[0].message(first), bindings[1].message(second)]
    );
    assert!(cold.unresolved_tool_calls.is_empty());
}
