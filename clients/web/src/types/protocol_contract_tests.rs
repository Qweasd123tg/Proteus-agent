use std::{collections::HashMap, path::PathBuf};

use proteus_contracts::{
    app_protocol as contract_protocol, contracts as contract_contracts, domain as contract_domain,
};
use serde::Serialize;
use serde_json::{Value, json};

use super::*;

fn decode_web_output(output: contract_protocol::StdioOutput) -> StdioOutput {
    let value = serde_json::to_value(output).expect("contract output JSON");
    serde_json::from_value(value).expect("web output decodes")
}

fn endpoint_body<T: Serialize>(request: T) -> Value {
    let mut value = serde_json::to_value(request).expect("request JSON");
    let Value::Object(fields) = &mut value else {
        panic!("contract request must serialize to an object");
    };
    fields.remove("type");
    value
}

fn assert_endpoint_body_matches_contract<T: Serialize, U: Serialize>(
    web_request: T,
    contract_request: U,
) {
    assert_eq!(
        serde_json::to_value(web_request).expect("web request JSON"),
        endpoint_body(contract_request)
    );
}

fn contract_approval_request() -> contract_protocol::AppApprovalRequest {
    contract_protocol::AppApprovalRequest::new(
        "approval-1".to_owned(),
        contract_domain::ToolCall::new(
            "call-1",
            "write_file",
            json!({ "path": "README.md", "content": "hello" }),
        ),
        PathBuf::from("/workspace"),
        "Need write access".to_owned(),
        Some(
            contract_domain::ToolSpec::new(
                "write_file",
                "Writes a file",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    }
                }),
                contract_domain::ToolSafety::WritesFiles,
            )
            .with_metadata(json!({ "approval_cache_scope": "workspace_write" })),
        ),
    )
    .with_preview(Some(
        contract_protocol::AppApprovalPreview::new(
            "write_file",
            "Write README.md",
            "Updates README.md",
        )
        .with_affected_files(vec!["README.md".to_owned()])
        .with_body("hello", "text")
        .with_metadata(json!({ "operation": "update" })),
    ))
    .with_origin(Some(
        contract_contracts::RequestOrigin::new(
            contract_domain::new_thread_id(),
            contract_domain::new_turn_id(),
        )
        .with_label("explore"),
    ))
    .with_seq(7)
}

fn contract_user_input_request() -> contract_contracts::UserInputRequest {
    contract_contracts::UserInputRequest::new(
        "input-1",
        PathBuf::from("/workspace"),
        vec![
            contract_contracts::UserInputQuestion::new(
                "scope",
                "Scope",
                "What should Proteus change?",
                vec![
                    contract_contracts::UserInputQuestionOption::new(
                        "Minimal",
                        "Only the direct fix",
                    )
                    .with_preview("recommended"),
                    contract_contracts::UserInputQuestionOption::new(
                        "Broad",
                        "Include nearby cleanup",
                    ),
                ],
            )
            .with_other(false)
            .with_secret(false)
            .with_multi_select(true),
        ],
    )
    .with_title("Choose implementation scope")
    .with_origin(
        contract_contracts::RequestOrigin::new(
            contract_domain::new_thread_id(),
            contract_domain::new_turn_id(),
        )
        .with_label("explore"),
    )
    .with_seq(9)
}

fn contract_response() -> contract_contracts::UserInputResponse {
    contract_contracts::UserInputResponse::new(HashMap::from([(
        "scope".to_owned(),
        contract_contracts::UserInputAnswer::new(vec!["Minimal".to_owned()]),
    )]))
}

#[test]
fn web_decodes_contract_stdio_output_events() {
    let session_id = contract_domain::new_session_id();
    let thread_id = contract_domain::new_thread_id();
    let events = vec![
        contract_protocol::AppServerEvent::Runtime {
            envelope: Box::new(contract_domain::EventEnvelope::new(
                contract_domain::EventContext::new(session_id, thread_id, None),
                7,
                contract_domain::Event::SessionStarted {
                    session_id,
                    cwd: PathBuf::from("/workspace"),
                    model: None,
                    session_dir: None,
                },
            )),
        },
        contract_protocol::AppServerEvent::UserMessageSubmitted {
            text: "hello".to_owned(),
        },
        contract_protocol::AppServerEvent::TurnOutput {
            output: Box::new(contract_domain::AgentOutput::new(
                "done",
                json!({ "ok": true }),
            )),
        },
        contract_protocol::AppServerEvent::ApprovalRequested {
            request: Box::new(contract_approval_request()),
        },
        contract_protocol::AppServerEvent::ApprovalResolved {
            approval_id: "approval-1".to_owned(),
            approved: true,
        },
        contract_protocol::AppServerEvent::UserInputRequested {
            request: Box::new(contract_user_input_request()),
        },
        contract_protocol::AppServerEvent::UserInputResolved {
            request_id: "input-1".to_owned(),
        },
        contract_protocol::AppServerEvent::SessionActivityUpdated {
            session_dir: PathBuf::from("/workspace/session-1"),
            activity: contract_protocol::AppSessionActivity::from_counts(1, 0, 0),
        },
        contract_protocol::AppServerEvent::ModulesReloaded {
            old_epoch: 1,
            new_epoch: 2,
            tool_names: vec!["read_file".to_owned()],
        },
        contract_protocol::AppServerEvent::Error {
            message: "boom".to_owned(),
        },
        contract_protocol::AppServerEvent::EventStreamLagged { count: 42 },
        contract_protocol::AppServerEvent::Shutdown,
    ];

    for event in events {
        let output = decode_web_output(contract_protocol::StdioOutput::Event {
            event: Box::new(event),
        });

        let StdioOutput::Event { event } = output else {
            panic!("unexpected output: {output:?}");
        };

        match *event {
            AppServerEvent::Runtime { envelope } => assert_eq!(envelope["seq"], 7),
            AppServerEvent::UserMessageSubmitted { text } => assert_eq!(text, "hello"),
            AppServerEvent::TurnOutput { output } => {
                assert_eq!(output["text"], "done");
                assert_eq!(output["metadata"]["ok"], true);
            }
            AppServerEvent::ApprovalRequested { request } => {
                assert_eq!(request.approval_id, "approval-1");
                assert_eq!(request.call.name, "write_file");
                assert_eq!(request.cwd, "/workspace");
                let preview = request.preview.expect("approval preview");
                assert_eq!(preview.kind, "write_file");
                assert_eq!(preview.affected_files, vec!["README.md"]);
                assert_eq!(preview.metadata["operation"], "update");
                let origin = request.origin.expect("approval origin");
                assert_eq!(origin.label.as_deref(), Some("explore"));
                assert!(!origin.thread_id.is_empty());
                assert!(!origin.turn_id.is_empty());
                assert_eq!(request.seq, 7);
            }
            AppServerEvent::ApprovalResolved {
                approval_id,
                approved,
            } => {
                assert_eq!(approval_id, "approval-1");
                assert!(approved);
            }
            AppServerEvent::UserInputRequested { request } => {
                assert_eq!(request.request_id, "input-1");
                assert_eq!(
                    request.title.as_deref(),
                    Some("Choose implementation scope")
                );
                let question = request.questions.first().expect("question");
                assert!(question.multi_select);
                assert_eq!(
                    question
                        .options
                        .first()
                        .and_then(|option| option.preview.as_deref()),
                    Some("recommended")
                );
                let origin = request.origin.expect("user input origin");
                assert_eq!(origin.label.as_deref(), Some("explore"));
                assert!(!origin.thread_id.is_empty());
                assert!(!origin.turn_id.is_empty());
                assert_eq!(request.seq, 9);
            }
            AppServerEvent::UserInputResolved { request_id } => {
                assert_eq!(request_id, "input-1")
            }
            AppServerEvent::ModulesReloaded {
                old_epoch,
                new_epoch,
                tool_names,
            } => {
                assert_eq!(old_epoch, 1);
                assert_eq!(new_epoch, 2);
                assert_eq!(tool_names, ["read_file"]);
            }
            AppServerEvent::SessionActivityUpdated {
                session_dir,
                activity,
            } => {
                assert_eq!(session_dir, "/workspace/session-1");
                assert_eq!(activity.status, "running");
                assert_eq!(activity.running_turns, 1);
            }
            AppServerEvent::Error { message } => assert_eq!(message, "boom"),
            AppServerEvent::EventStreamLagged { count } => assert_eq!(count, 42),
            AppServerEvent::Shutdown => {}
        }
    }
}

#[test]
fn web_decodes_contract_pending_requests() {
    let pending = contract_protocol::AppPendingRequests::new(
        vec![contract_approval_request()],
        vec![contract_user_input_request()],
    );

    let value = serde_json::to_value(pending).expect("pending JSON");
    let decoded: PendingControlPlaneInfo =
        serde_json::from_value(value).expect("web pending requests");

    assert_eq!(decoded.approvals.len(), 1);
    assert_eq!(decoded.approvals[0].approval_id, "approval-1");
    assert_eq!(decoded.approvals[0].call.args["path"], "README.md");
    assert_eq!(decoded.user_inputs.len(), 1);
    assert_eq!(decoded.user_inputs[0].request_id, "input-1");
    assert_eq!(
        decoded.user_inputs[0].questions[0].options[0].label,
        "Minimal"
    );
}

#[test]
fn web_decodes_contract_session_summary() {
    let session_id = contract_domain::new_session_id();
    let summary = contract_protocol::AppSessionSummary::new(
        PathBuf::from("/tmp/session-1"),
        session_id,
        PathBuf::from("/workspace"),
        3,
        Some(42),
        Some("first message".to_owned()),
    )
    .with_activity(
        contract_protocol::AppSessionActivity::from_running_turn_ids(
            vec!["turn-1".to_owned()],
            0,
            0,
        ),
    );

    let value = serde_json::to_value(summary).expect("contract session summary JSON");
    let decoded: SessionSummary = serde_json::from_value(value).expect("web session summary");

    assert_eq!(decoded.session_dir, "/tmp/session-1");
    assert_eq!(decoded.session_id, session_id.to_string());
    assert_eq!(decoded.workspace_path, "/workspace");
    assert_eq!(decoded.message_count, 3);
    assert_eq!(decoded.updated_at_ms, Some(42));
    assert_eq!(decoded.preview.as_deref(), Some("first message"));
    let activity = decoded.activity.expect("live activity");
    assert_eq!(activity.status, "running");
    assert_eq!(activity.running_turn_ids, ["turn-1"]);
}

#[test]
fn web_endpoint_request_bodies_match_contract_stdio_requests_without_transport_tag() {
    assert_endpoint_body_matches_contract(
        SendRequest {
            id: Some("send-1".to_owned()),
            text: "hello".to_owned(),
            session_dir: None,
        },
        contract_protocol::StdioRequest::Send {
            id: Some("send-1".to_owned()),
            text: "hello".to_owned(),
        },
    );
    assert_endpoint_body_matches_contract(
        SetPermissionModeRequest {
            id: Some("mode-1".to_owned()),
            mode: PermissionMode::Auto,
            session_dir: None,
        },
        contract_protocol::StdioRequest::SetPermissionMode {
            id: Some("mode-1".to_owned()),
            mode: contract_domain::PermissionMode::Auto,
        },
    );
    assert_endpoint_body_matches_contract(
        SetModelRequest {
            id: Some("model-1".to_owned()),
            model: "gpt-5".to_owned(),
            session_dir: None,
        },
        contract_protocol::StdioRequest::SetModel {
            id: Some("model-1".to_owned()),
            model: "gpt-5".to_owned(),
        },
    );
    assert_endpoint_body_matches_contract(
        SetReasoningEffortRequest {
            id: Some("effort-1".to_owned()),
            effort: Some("high".to_owned()),
            session_dir: None,
        },
        contract_protocol::StdioRequest::SetReasoningEffort {
            id: Some("effort-1".to_owned()),
            effort: Some("high".to_owned()),
        },
    );
    // «none» — валидное значение effort: выключает рассуждения (см.
    // Runtime::set_reasoning_effort); отдельного /reasoning клиент не зовёт.
    assert_endpoint_body_matches_contract(
        SetReasoningEffortRequest {
            id: Some("effort-2".to_owned()),
            effort: Some("none".to_owned()),
            session_dir: None,
        },
        contract_protocol::StdioRequest::SetReasoningEffort {
            id: Some("effort-2".to_owned()),
            effort: Some("none".to_owned()),
        },
    );
    assert_endpoint_body_matches_contract(
        ResolveApprovalRequest {
            id: Some("approval-1".to_owned()),
            approval_id: "approval-1".to_owned(),
            approved: true,
            note: Some("ok".to_owned()),
            cache: ApprovalCacheScope::WorkspaceWrite,
        },
        contract_protocol::StdioRequest::Approval {
            id: Some("approval-1".to_owned()),
            approval_id: "approval-1".to_owned(),
            approved: true,
            note: Some("ok".to_owned()),
            cache: contract_contracts::ApprovalCacheScope::WorkspaceWrite,
        },
    );
    assert_endpoint_body_matches_contract(
        UserInputSubmitRequest {
            id: Some("input-1".to_owned()),
            request_id: "input-1".to_owned(),
            response: UserInputResponseBody {
                answers: HashMap::from([(
                    "scope".to_owned(),
                    UserInputAnswerBody {
                        answers: vec!["Minimal".to_owned()],
                    },
                )]),
            },
        },
        contract_protocol::StdioRequest::UserInput {
            id: Some("input-1".to_owned()),
            request_id: "input-1".to_owned(),
            response: contract_response(),
        },
    );
    assert_endpoint_body_matches_contract(
        CancelRequest {
            id: Some("cancel-1".to_owned()),
            target_id: "send-1".to_owned(),
        },
        contract_protocol::StdioRequest::Cancel {
            id: Some("cancel-1".to_owned()),
            target_id: "send-1".to_owned(),
        },
    );
}

#[test]
fn web_http_only_session_request_shapes_stay_stable() {
    assert_eq!(
        serde_json::to_value(ResumeSessionRequest {
            id: Some("resume-1".to_owned()),
            session_dir: "/tmp/proteus-session".to_owned(),
        })
        .expect("resume request JSON"),
        json!({
            "id": "resume-1",
            "session_dir": "/tmp/proteus-session"
        })
    );
    assert_eq!(
        serde_json::to_value(DeleteSessionRequest {
            id: Some("delete-1".to_owned()),
            session_dir: "/tmp/proteus-session".to_owned(),
        })
        .expect("delete request JSON"),
        json!({
            "id": "delete-1",
            "session_dir": "/tmp/proteus-session"
        })
    );
}
