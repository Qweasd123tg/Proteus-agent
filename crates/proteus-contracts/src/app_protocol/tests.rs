use std::path::PathBuf;

use serde_json::json;

use super::*;
use crate::domain::{
    Event, EventContext, EventEnvelope, ToolCall, new_call_id, new_execution_id, new_session_id,
    new_thread_id, new_turn_id,
};

#[test]
fn snapshots_require_complete_revisions() {
    fn required<T: serde::Serialize + serde::de::DeserializeOwned>(snapshot: T, fields: &[&str]) {
        let value = serde_json::to_value(snapshot).unwrap();
        for field in fields {
            let mut incomplete = value.clone();
            incomplete.as_object_mut().unwrap().remove(*field);
            assert!(
                serde_json::from_value::<T>(incomplete).is_err(),
                "missing {field}"
            );
        }
    }
    required(
        AppPendingRequests::new(new_session_id(), "live".into()),
        &[
            "session_id",
            "stream_id",
            "seq",
            "approvals",
            "user_inputs",
            "queued_user_messages",
        ],
    );
    required(
        AppSessionSnapshot {
            session_id: new_session_id(),
            stream_id: "live".into(),
            seq: 0,
            root_thread_id: None,
            transcript: vec![],
            execution: Default::default(),
        },
        &["session_id", "stream_id", "seq", "transcript", "execution"],
    );
}

#[test]
fn approval_request_rejects_incomplete_wire_payload() {
    let payload = json!({
        "approval_id": "approval-1",
        "call": {
            "id": "call-1",
            "name": "shell",
            "args": { "command": "cargo test" }
        },
        "cwd": "/workspace",
        "reason": "test approval",
        "tool_spec": null
    });

    serde_json::from_value::<AppApprovalRequest>(payload)
        .expect_err("incomplete approval request must fail");
}

#[test]
fn line_client_commands_use_strict_tagged_wire_shapes() {
    let history: StdioRequest = serde_json::from_value(json!({
        "type": "history_summary",
        "id": "history-1"
    }))
    .expect("history request");
    assert_eq!(history.id().as_deref(), Some("history-1"));

    let remember: StdioRequest = serde_json::from_value(json!({
        "type": "remember",
        "id": "remember-1",
        "kind": "preference",
        "content": "use protocol"
    }))
    .expect("remember request");
    assert_eq!(remember.id().as_deref(), Some("remember-1"));

    let result = AppRememberResult::new("preference", "use protocol");
    assert_eq!(
        serde_json::to_value(result).expect("remember result"),
        json!({"kind": "preference", "content": "use protocol"})
    );
}

/// Attribution и порядок очереди переживают wire-сериализацию.
#[test]
fn approval_request_roundtrips_origin_and_seq() {
    let origin = crate::contracts::RequestOrigin::for_turn(
        new_execution_id(),
        new_thread_id(),
        new_turn_id(),
    )
    .with_label("explore");
    let request = AppApprovalRequest::new(
        "approval-1".to_owned(),
        ToolCall::new(new_call_id(), "shell", json!({ "command": "cargo test" })),
        PathBuf::from("/workspace"),
        "test approval".to_owned(),
        None,
    )
    .with_origin(Some(origin.clone()))
    .with_seq(42);

    let payload = serde_json::to_value(&request).expect("serialize approval request");
    let parsed: AppApprovalRequest =
        serde_json::from_value(payload).expect("parse approval request");

    assert_eq!(parsed.origin, Some(origin));
    assert_eq!(parsed.seq, 42);
}

/// User-input запросы несут ту же attribution/queue-position схему, что
/// approvals.
#[test]
fn user_input_request_roundtrips_origin_and_seq() {
    let origin = crate::contracts::RequestOrigin::for_turn(
        new_execution_id(),
        new_thread_id(),
        new_turn_id(),
    )
    .with_label("explore");
    let request = UserInputRequest::new("input-1", PathBuf::from("/workspace"), Vec::new())
        .with_origin(origin.clone())
        .with_seq(7);

    let payload = serde_json::to_value(&request).expect("serialize user input request");
    let parsed: UserInputRequest =
        serde_json::from_value(payload).expect("parse user input request");
    assert_eq!(parsed.origin, Some(origin));
    assert_eq!(parsed.seq, 7);
}

#[test]
fn approval_request_roundtrips_preview() {
    let request = AppApprovalRequest::new(
        "approval-1".to_owned(),
        ToolCall::new(
            new_call_id(),
            "write_file",
            json!({ "path": "a.txt", "content": "hello" }),
        ),
        PathBuf::from("/workspace"),
        "test approval".to_owned(),
        None,
    )
    .with_preview(Some(
        AppApprovalPreview::new("write_file", "File write preview", "Create a.txt")
            .with_affected_files(vec!["a.txt".to_owned()])
            .with_body("hello", "text")
            .with_metadata(json!({ "operation": "create" })),
    ));

    let value = serde_json::to_value(&request).expect("serialize request");
    let decoded: AppApprovalRequest = serde_json::from_value(value).expect("decode request");

    let preview = decoded.preview.expect("preview");
    assert_eq!(preview.kind, "write_file");
    assert_eq!(preview.affected_files, vec!["a.txt"]);
    assert_eq!(preview.metadata["operation"], "create");
}

#[test]
fn boxed_app_server_event_keeps_wire_shape() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let event = AppServerEvent::Runtime {
        envelope: Box::new(EventEnvelope::new(
            EventContext::new(session_id, thread_id, None),
            1,
            Event::SessionStarted {
                session_id,
                cwd: PathBuf::from("/workspace"),
                model: None,
                session_dir: None,
            },
        )),
    };

    let value = serde_json::to_value(event).expect("event JSON");

    assert_eq!(value["type"], "runtime");
    assert_eq!(value["envelope"]["seq"], 1);

    let decoded: AppServerEvent = serde_json::from_value(value).expect("decode event");
    match decoded {
        AppServerEvent::Runtime { envelope } => assert_eq!(envelope.seq, 1),
        other => panic!("expected runtime event, got {other:?}"),
    }
}

#[test]
fn session_activity_uses_stable_status_order() {
    assert_eq!(
        AppSessionActivity::from_counts(0, 0, 0).status,
        AppSessionActivityStatus::Idle
    );
    assert_eq!(
        AppSessionActivity::from_counts(1, 0, 0).status,
        AppSessionActivityStatus::Running
    );
    assert_eq!(
        AppSessionActivity::from_counts(1, 1, 0).status,
        AppSessionActivityStatus::WaitingApproval
    );
    assert_eq!(
        AppSessionActivity::from_counts(1, 1, 1).status,
        AppSessionActivityStatus::WaitingInput
    );
}

#[test]
fn session_activity_status_stays_string_on_wire() {
    let activity = AppSessionActivity::from_counts(1, 0, 0);
    let value = serde_json::to_value(activity).expect("activity JSON");

    assert_eq!(value["status"], "running");
}

#[test]
fn session_activity_can_carry_running_run_ids() {
    let activity = AppSessionActivity::from_running_run_ids(
        vec!["run-2".to_owned(), "run-1".to_owned()],
        0,
        0,
    );
    let value = serde_json::to_value(&activity).expect("activity JSON");

    assert_eq!(activity.running_runs, 2);
    assert_eq!(
        value["running_run_ids"],
        serde_json::json!(["run-2", "run-1"])
    );

    let decoded: AppSessionActivity = serde_json::from_value(value).expect("activity decode");
    assert_eq!(
        decoded.running_run_ids,
        vec!["run-2".to_owned(), "run-1".to_owned()]
    );
}

#[test]
fn session_activity_status_rejects_unknown_wire_value() {
    serde_json::from_value::<AppSessionActivity>(serde_json::json!({
        "status": "paused",
        "running_runs": 0,
        "running_run_ids": [],
        "pending_approvals": 0,
        "pending_user_inputs": 0,
    }))
    .expect_err("unknown activity status must fail");
}

#[test]
fn session_activity_rejects_legacy_turn_shaped_transport_fields() {
    serde_json::from_value::<AppSessionActivity>(serde_json::json!({
        "status": "idle",
        "running_turns": 0,
        "running_turn_ids": [],
        "pending_approvals": 0,
        "pending_user_inputs": 0,
    }))
    .expect_err("transport run identity must not be accepted as a domain turn");
}
