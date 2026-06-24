use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use coding_workflow::CodingPlanExecuteReviewWorkflow;
use context_pack::SimpleContextBuilderPlugin;
use policy_pack::AskWritePolicyPlugin;
use proteus_contracts::{
    abi_stable::sabi_trait::TD_Opaque,
    contracts::Renderer_TO,
    plugin::{PluginApprovalPolicy_TO, PluginContextBuilder_TO, PluginWorkflow_TO},
};
use renderer_pack::PlainRendererPlugin;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};

use super::*;
use crate::{
    contracts::{ApprovalRequest, UserInputQuestion, UserInputQuestionOption, UserInputRequest},
    core::{PendingApproval, PendingUserInput, SessionStore},
    domain::{Event, PermissionMode, ToolCall, new_call_id, new_session_id},
    model_standard::{CanonicalMessage, MessageRole},
};

fn test_catalog() -> BuiltinModuleCatalog {
    let mut catalog = BuiltinModuleCatalog::new();
    catalog
        .register_plugin_context_builder(
            "simple",
            PluginContextBuilder_TO::from_value(SimpleContextBuilderPlugin, TD_Opaque),
        )
        .expect("register test context builder");
    catalog
        .register_plugin_workflow(
            "coding.plan_execute_review",
            PluginWorkflow_TO::from_value(CodingPlanExecuteReviewWorkflow, TD_Opaque),
        )
        .expect("register test workflow");
    catalog
        .register_plugin_policy(
            "ask_write",
            PluginApprovalPolicy_TO::from_value(AskWritePolicyPlugin, TD_Opaque),
        )
        .expect("register test policy");
    catalog
        .register_plugin_renderer(
            "plain",
            Renderer_TO::from_value(PlainRendererPlugin, TD_Opaque),
        )
        .expect("register test renderer");
    catalog
}

fn pending_approval_entry(
    approval_id: &str,
    responder: oneshot::Sender<ApprovalResponse>,
) -> PendingApprovalEntry {
    PendingApprovalEntry {
        request: AppApprovalRequest::new(
            approval_id.to_owned(),
            ToolCall::new(new_call_id(), "write_file", serde_json::json!({})),
            PathBuf::from("."),
            "test approval".to_owned(),
            None,
        ),
        responder,
    }
}

fn pending_user_input_entry(
    request_id: &str,
    responder: oneshot::Sender<UserInputResponse>,
) -> PendingUserInputEntry {
    PendingUserInputEntry {
        request: UserInputRequest::new(request_id.to_owned(), PathBuf::from("."), Vec::new()),
        responder,
    }
}

#[test]
fn apply_patch_approval_preview_extracts_affected_files() {
    let call = ToolCall::new(
        new_call_id(),
        "apply_patch",
        serde_json::json!({
            "patch": "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-old\n+new\n*** Add File: notes.txt\n+hello\n*** End Patch\n"
        }),
    );

    let preview =
        approval_preview_for(&call, Path::new("/workspace")).expect("apply_patch preview");

    assert_eq!(preview.kind, "patch");
    assert_eq!(preview.affected_files, vec!["notes.txt", "src/main.rs"]);
    assert!(preview.summary.contains("2 files"));
    assert!(
        preview
            .body
            .unwrap()
            .contains("*** Update File: src/main.rs")
    );
}

#[test]
fn apply_patch_approval_preview_accepts_freeform_input() {
    let call = ToolCall::new(
        new_call_id(),
        "apply_patch",
        serde_json::json!({
            "input": "*** Begin Patch\n*** Add File: codex.txt\n+hello\n*** End Patch\n"
        }),
    );

    let preview =
        approval_preview_for(&call, Path::new("/workspace")).expect("apply_patch preview");

    assert_eq!(preview.kind, "patch");
    assert_eq!(preview.affected_files, vec!["codex.txt"]);
    assert!(preview.body.unwrap().contains("*** Add File: codex.txt"));
}

#[test]
fn write_file_approval_preview_shows_overwrite_diff() {
    let cwd = tempfile::tempdir().expect("cwd");
    std::fs::write(cwd.path().join("sample.txt"), "one\ntwo\n").expect("sample file");
    let call = ToolCall::new(
        new_call_id(),
        "write_file",
        serde_json::json!({
            "path": "sample.txt",
            "content": "one\nthree\n"
        }),
    );

    let preview = approval_preview_for(&call, cwd.path()).expect("write_file preview");

    assert_eq!(preview.kind, "write_file");
    assert_eq!(preview.affected_files, vec!["sample.txt"]);
    assert!(preview.summary.contains("Overwrite sample.txt"));
    let body = preview.body.unwrap();
    assert!(body.contains("-two"));
    assert!(body.contains("+three"));
}

#[test]
fn write_file_approval_preview_shows_create_body() {
    let cwd = tempfile::tempdir().expect("cwd");
    let call = ToolCall::new(
        new_call_id(),
        "write_file",
        serde_json::json!({
            "path": "new.txt",
            "content": "hello\n"
        }),
    );

    let preview = approval_preview_for(&call, cwd.path()).expect("write_file preview");

    assert_eq!(preview.kind, "write_file");
    assert_eq!(preview.affected_files, vec!["new.txt"]);
    assert!(preview.summary.contains("Create new.txt"));
    assert_eq!(preview.language.as_deref(), Some("text"));
    assert_eq!(preview.body.as_deref(), Some("hello\n"));
    assert_eq!(preview.metadata["operation"], "create");
}

#[test]
fn write_file_approval_preview_does_not_read_outside_workspace() {
    let cwd = tempfile::tempdir().expect("cwd");
    let outside = tempfile::NamedTempFile::new().expect("outside file");
    std::fs::write(outside.path(), "secret outside content\n").expect("outside content");
    let outside_path = outside.path().display().to_string();
    let call = ToolCall::new(
        new_call_id(),
        "write_file",
        serde_json::json!({
            "path": outside_path,
            "content": "replacement\n"
        }),
    );

    let preview = approval_preview_for(&call, cwd.path()).expect("write_file preview");

    assert_eq!(preview.kind, "write_file");
    assert_eq!(preview.language.as_deref(), Some("text"));
    assert_eq!(preview.body.as_deref(), Some("replacement\n"));
    assert_eq!(preview.metadata["operation"], "write");
    assert_eq!(preview.metadata["workspace_scoped"], false);
}

#[test]
fn shell_approval_preview_uses_exact_command_metadata() {
    let call = ToolCall::new(
        new_call_id(),
        "shell",
        serde_json::json!({ "command": "cargo test" }),
    );

    let preview = approval_preview_for(&call, Path::new("/workspace")).expect("shell preview");

    assert_eq!(preview.kind, "command");
    assert_eq!(preview.language.as_deref(), Some("shell"));
    assert_eq!(preview.body.as_deref(), Some("cargo test"));
    assert_eq!(preview.metadata["cache_scope"], "exact_command");
}

#[tokio::test]
async fn app_server_updates_permission_mode_without_restart() {
    let cwd = tempfile::tempdir().expect("cwd");
    let mut config = AppConfig::default();
    config.permissions.mode = PermissionMode::Normal;
    let server = AgentAppServer::launch_with_module_catalog(
        config,
        cwd.path().to_path_buf(),
        None,
        test_catalog(),
    )
    .expect("app server");

    assert_eq!(server.permission_mode().await, PermissionMode::Normal);

    server.set_permission_mode(PermissionMode::Plan).await;

    assert_eq!(server.permission_mode().await, PermissionMode::Plan);
}

#[tokio::test]
async fn approval_forwarder_keeps_request_when_no_client_can_receive_event() {
    let (approval_tx, approval_rx) = mpsc::channel(1);
    let (events, _) = broadcast::channel(1);
    let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
    spawn_approval_forwarder(
        approval_rx,
        events,
        pending_approvals.clone(),
        Duration::from_secs(60),
    );

    let (responder, mut response_rx) = oneshot::channel();
    approval_tx
        .send(PendingApproval {
            request: ApprovalRequest::new(
                ToolCall::new(new_call_id(), "write_file", serde_json::json!({})),
                PathBuf::from("."),
                "test approval",
                None,
            ),
            responder,
        })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(30)).await;

    assert_eq!(pending_approvals.lock().await.len(), 1);
    assert!(response_rx.try_recv().is_err());
}

#[tokio::test]
async fn approval_forwarder_denies_when_client_does_not_answer_before_timeout() {
    let (approval_tx, approval_rx) = mpsc::channel(1);
    let (events, _) = broadcast::channel(8);
    let mut event_rx = events.subscribe();
    let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
    spawn_approval_forwarder(
        approval_rx,
        events,
        pending_approvals.clone(),
        Duration::from_millis(20),
    );

    let (responder, response_rx) = oneshot::channel();
    approval_tx
        .send(PendingApproval {
            request: ApprovalRequest::new(
                ToolCall::new(new_call_id(), "write_file", serde_json::json!({})),
                PathBuf::from("."),
                "test approval",
                None,
            ),
            responder,
        })
        .await
        .unwrap();

    let request_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("approval request event should arrive")
        .expect("event stream should stay open");
    let approval_id = match request_event {
        AppServerEvent::ApprovalRequested { request } => request.approval_id,
        other => panic!("expected approval request, got {other:?}"),
    };

    let response = tokio::time::timeout(Duration::from_secs(1), response_rx)
        .await
        .expect("approval response should not hang")
        .expect("approval responder should send denial");

    assert!(!response.approved);
    assert!(
        response
            .note
            .as_deref()
            .is_some_and(|note| note.contains("timed out"))
    );
    assert!(pending_approvals.lock().await.is_empty());

    let resolved_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("approval resolved event should arrive")
        .expect("event stream should stay open");
    assert!(matches!(
        resolved_event,
        AppServerEvent::ApprovalResolved {
            approval_id: id,
            approved: false,
        } if id == approval_id
    ));
}

#[tokio::test]
async fn approval_forwarder_waits_without_timeout_when_timeout_is_zero() {
    let (approval_tx, approval_rx) = mpsc::channel(1);
    let (events, _) = broadcast::channel(8);
    let mut event_rx = events.subscribe();
    let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
    spawn_approval_forwarder(
        approval_rx,
        events,
        pending_approvals.clone(),
        Duration::ZERO,
    );

    let (responder, mut response_rx) = oneshot::channel();
    approval_tx
        .send(PendingApproval {
            request: ApprovalRequest::new(
                ToolCall::new(new_call_id(), "write_file", serde_json::json!({})),
                PathBuf::from("."),
                "test approval",
                None,
            ),
            responder,
        })
        .await
        .unwrap();

    let request_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("approval request event should arrive")
        .expect("event stream should stay open");
    let approval_id = match request_event {
        AppServerEvent::ApprovalRequested { request } => request.approval_id,
        other => panic!("expected approval request, got {other:?}"),
    };

    tokio::time::sleep(Duration::from_millis(30)).await;

    assert!(pending_approvals.lock().await.contains_key(&approval_id));
    assert!(response_rx.try_recv().is_err());
}

#[tokio::test]
async fn user_input_forwarder_waits_without_timeout_when_timeout_is_zero() {
    let (user_input_tx, user_input_rx) = mpsc::channel(1);
    let (events, _) = broadcast::channel(8);
    let mut event_rx = events.subscribe();
    let pending_user_inputs = Arc::new(Mutex::new(HashMap::new()));
    spawn_user_input_forwarder(
        user_input_rx,
        events,
        pending_user_inputs.clone(),
        Duration::ZERO,
    );

    let request_id = "question-1".to_owned();
    let (responder, mut response_rx) = oneshot::channel();
    user_input_tx
        .send(PendingUserInput {
            request: UserInputRequest::new(
                request_id.clone(),
                PathBuf::from("."),
                vec![UserInputQuestion::new(
                    "scope",
                    "Scope",
                    "Which scope?",
                    vec![UserInputQuestionOption::new("Small", "Small scope")],
                )],
            ),
            responder,
        })
        .await
        .unwrap();

    let request_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("user input request event should arrive")
        .expect("event stream should stay open");
    assert!(matches!(
        request_event,
        AppServerEvent::UserInputRequested { request } if request.request_id == request_id
    ));

    tokio::time::sleep(Duration::from_millis(30)).await;

    assert!(pending_user_inputs.lock().await.contains_key(&request_id));
    assert!(response_rx.try_recv().is_err());
}

#[tokio::test]
async fn shutdown_denies_pending_approvals() {
    let (events, _) = broadcast::channel(8);
    let mut event_rx = events.subscribe();
    let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
    let (responder, response_rx) = oneshot::channel();
    let approval_id = "approval-1".to_owned();
    pending_approvals.lock().await.insert(
        approval_id.clone(),
        pending_approval_entry(&approval_id, responder),
    );

    deny_pending_approvals(
        pending_approvals.clone(),
        events,
        "app-server shutting down".to_owned(),
    )
    .await;

    let response = response_rx
        .await
        .expect("shutdown should send approval response");
    assert!(!response.approved);
    assert_eq!(response.note.as_deref(), Some("app-server shutting down"));
    assert!(pending_approvals.lock().await.is_empty());

    let resolved_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("approval resolved event should arrive")
        .expect("event stream should stay open");
    assert!(matches!(
        resolved_event,
        AppServerEvent::ApprovalResolved {
            approval_id: id,
            approved: false,
        } if id == approval_id
    ));
}

#[tokio::test]
async fn shutdown_resolves_pending_user_inputs() {
    let (events, _) = broadcast::channel(8);
    let mut event_rx = events.subscribe();
    let pending_user_inputs = Arc::new(Mutex::new(HashMap::new()));
    let (responder, response_rx) = oneshot::channel();
    let request_id = "input-1".to_owned();
    pending_user_inputs.lock().await.insert(
        request_id.clone(),
        pending_user_input_entry(&request_id, responder),
    );

    deny_pending_user_inputs(
        pending_user_inputs.clone(),
        events,
        "app-server shutting down".to_owned(),
    )
    .await;

    let response = response_rx
        .await
        .expect("shutdown should send user input response");
    assert!(response.answers.is_empty());
    assert!(pending_user_inputs.lock().await.is_empty());

    let resolved_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("user input resolved event should arrive")
        .expect("event stream should stay open");
    assert!(matches!(
        resolved_event,
        AppServerEvent::UserInputResolved { request_id: id } if id == request_id
    ));
}

#[tokio::test]
async fn cancel_pending_approvals_denies_pending_requests() {
    let cwd = tempfile::tempdir().expect("cwd");
    let mut config = AppConfig::default();
    config.modules.patch = "null".to_owned();
    let handle = AgentAppServer::launch_with_module_catalog(
        config,
        cwd.path().to_path_buf(),
        None,
        test_catalog(),
    )
    .expect("app server");
    let mut event_rx = handle.subscribe();
    let (responder, response_rx) = oneshot::channel();
    let approval_id = "approval-cancel".to_owned();
    handle.pending_approvals.lock().await.insert(
        approval_id.clone(),
        pending_approval_entry(&approval_id, responder),
    );

    handle
        .cancel_pending_approvals("turn canceled by client".to_owned())
        .await;

    let response = response_rx
        .await
        .expect("cancel should send approval response");
    assert!(!response.approved);
    assert_eq!(response.note.as_deref(), Some("turn canceled by client"));
    assert!(handle.pending_approvals.lock().await.is_empty());

    let resolved_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("approval resolved event should arrive")
        .expect("event stream should stay open");
    assert!(matches!(
        resolved_event,
        AppServerEvent::ApprovalResolved {
            approval_id: id,
            approved: false,
        } if id == approval_id
    ));

    handle.shutdown().await;
}

#[tokio::test]
async fn cancel_pending_user_inputs_resolves_pending_requests() {
    let cwd = tempfile::tempdir().expect("cwd");
    let mut config = AppConfig::default();
    config.modules.patch = "null".to_owned();
    let handle = AgentAppServer::launch_with_module_catalog(
        config,
        cwd.path().to_path_buf(),
        None,
        test_catalog(),
    )
    .expect("app server");
    let mut event_rx = handle.subscribe();
    let (responder, response_rx) = oneshot::channel();
    let request_id = "input-cancel".to_owned();
    handle.pending_user_inputs.lock().await.insert(
        request_id.clone(),
        pending_user_input_entry(&request_id, responder),
    );

    handle
        .cancel_pending_user_inputs("turn canceled by client".to_owned())
        .await;

    let response = response_rx
        .await
        .expect("cancel should send user input response");
    assert!(response.answers.is_empty());
    assert!(handle.pending_user_inputs.lock().await.is_empty());

    let resolved_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("user input resolved event should arrive")
        .expect("event stream should stay open");
    assert!(matches!(
        resolved_event,
        AppServerEvent::UserInputResolved { request_id: id } if id == request_id
    ));

    handle.shutdown().await;
}

#[tokio::test]
async fn zero_timeout_pending_user_input_resolves_on_shutdown() {
    let (user_input_tx, user_input_rx) = mpsc::channel(1);
    let (events, _) = broadcast::channel(8);
    let mut event_rx = events.subscribe();
    let pending_user_inputs = Arc::new(Mutex::new(HashMap::new()));
    spawn_user_input_forwarder(
        user_input_rx,
        events.clone(),
        pending_user_inputs.clone(),
        Duration::ZERO,
    );

    let request_id = "question-shutdown".to_owned();
    let (responder, response_rx) = oneshot::channel();
    user_input_tx
        .send(PendingUserInput {
            request: UserInputRequest::new(
                request_id.clone(),
                PathBuf::from("."),
                vec![UserInputQuestion::new(
                    "scope",
                    "Scope",
                    "Which scope?",
                    vec![UserInputQuestionOption::new("Small", "Small scope")],
                )],
            ),
            responder,
        })
        .await
        .unwrap();

    let request_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("user input request event should arrive")
        .expect("event stream should stay open");
    assert!(matches!(
        request_event,
        AppServerEvent::UserInputRequested { request } if request.request_id == request_id
    ));

    deny_pending_user_inputs(
        pending_user_inputs.clone(),
        events,
        "app-server shutting down".to_owned(),
    )
    .await;

    let response = tokio::time::timeout(Duration::from_secs(1), response_rx)
        .await
        .expect("user input response should not hang")
        .expect("user input responder should send empty response");
    assert!(response.answers.is_empty());
    assert!(pending_user_inputs.lock().await.is_empty());
}

#[tokio::test]
async fn app_server_forwards_streaming_text_deltas_before_turn_output() {
    let cwd = tempfile::tempdir().expect("cwd");
    let mut config = AppConfig::default();
    config.modules.workflow = "coding.plan_execute_review".to_owned();
    config.modules.context = "simple".to_owned();
    config.modules.policy = "ask_write".to_owned();
    config.modules.renderer = "plain".to_owned();
    config.modules.patch = "null".to_owned();

    let handle = AgentAppServer::launch_with_module_catalog(
        config,
        cwd.path().to_path_buf(),
        None,
        test_catalog(),
    )
    .expect("app server");
    let mut event_rx = handle.subscribe();
    let send_handle = handle.clone();
    let turn = tokio::spawn(async move {
        send_handle
            .send_user_message("stream this".to_owned())
            .await
            .expect("turn output")
    });

    let mut saw_delta = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("event should arrive")
            .expect("event stream should stay open");
        match event {
            AppServerEvent::Runtime { envelope } => {
                if matches!(envelope.event, Event::AssistantTextDelta { .. }) {
                    saw_delta = true;
                }
            }
            AppServerEvent::TurnOutput { .. } => break,
            AppServerEvent::Error { message } => {
                panic!("unexpected app-server error: {message}")
            }
            _ => {}
        }
    }

    let output = turn.await.expect("turn task");
    assert!(
        saw_delta,
        "expected at least one text delta before TurnOutput"
    );
    assert!(output.text.contains("Fake final answer"));
    handle.shutdown().await;
}

#[tokio::test]
async fn transcript_projects_runtime_history_for_resume_ui() {
    let cwd = tempfile::tempdir().expect("cwd");
    let mut config = AppConfig::default();
    config.modules.workflow = "coding.plan_execute_review".to_owned();
    config.modules.context = "simple".to_owned();
    config.modules.policy = "ask_write".to_owned();
    config.modules.renderer = "plain".to_owned();
    config.modules.patch = "null".to_owned();

    let handle = AgentAppServer::launch_with_module_catalog(
        config,
        cwd.path().to_path_buf(),
        None,
        test_catalog(),
    )
    .expect("app server");

    handle
        .send_user_message("restore this chat".to_owned())
        .await
        .expect("turn output");

    let transcript = handle.transcript().await;
    assert!(
        transcript
            .iter()
            .any(|message| message.role == "user" && message.text == "restore this chat")
    );
    assert!(transcript.iter().any(|message| {
        message.role == "assistant" && message.text.contains("Fake final answer")
    }));

    handle.shutdown().await;
}

#[tokio::test]
async fn config_summary_includes_current_session_dir_field() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let handle = AgentAppServer::launch_with_module_catalog(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        Some(&config_path),
        test_catalog(),
    )
    .expect("app server");

    let summary = handle.config_summary().await;

    let session_dir = summary
        .get("session_dir")
        .and_then(|value| value.as_str())
        .expect("session_dir");
    let expected = handle
        .runtime
        .session_dir()
        .expect("runtime session dir")
        .display()
        .to_string();
    assert_eq!(session_dir, expected);
    handle.shutdown().await;
}

#[tokio::test]
async fn launch_or_resume_latest_uses_last_non_empty_workspace_session() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    let saved_session_id = new_session_id();
    let saved_store = SessionStore::new(config_dir.path(), cwd.path(), saved_session_id);
    saved_store
        .append_messages(&[CanonicalMessage::text(
            MessageRole::User,
            "restore saved chat",
        )])
        .await
        .expect("append saved messages");

    let empty_store = SessionStore::new(config_dir.path(), cwd.path(), new_session_id());
    empty_store
        .materialize()
        .await
        .expect("materialize empty session");

    let handle = AgentAppServer::launch_or_resume_latest(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .expect("app server");

    assert_eq!(
        handle.runtime.session_dir(),
        Some(saved_store.session_dir())
    );
    assert_eq!(handle.runtime.history().await.len(), 1);
    assert_eq!(
        handle.transcript().await[0].text,
        "restore saved chat".to_owned()
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn reload_tools_rebuilds_registry_from_config_path_and_emits_event() {
    let cwd = tempfile::tempdir().expect("cwd");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        r#"
[tools]
enabled = []
"#,
    )
    .expect("initial config");
    let config = AppConfig::load(Some(&config_path))
        .await
        .expect("load initial config");
    let handle = AgentAppServer::launch(config, cwd.path().to_path_buf(), Some(&config_path))
        .expect("app server");
    let mut event_rx = handle.subscribe();

    std::fs::write(
        &config_path,
        r#"
[modules]
workflow = "missing_after_reload"

[tools]
enabled = []

[[tools.configured]]
name = "reload_probe"
description = "Probe tool added by reload"
safety = "ReadOnly"

[tools.configured.executor]
kind = "process"
command = "printf"
args = ["ok"]
"#,
    )
    .expect("updated config");

    let report = handle.reload_tools().await.expect("reload tools");
    assert_eq!(report.old_epoch, 0);
    assert_eq!(report.new_epoch, 1);
    assert!(report.tool_names.iter().any(|name| name == "reload_probe"));

    let reload_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("reload event should arrive")
        .expect("event stream should stay open");
    assert!(matches!(
        reload_event,
        AppServerEvent::ModulesReloaded {
            old_epoch: 0,
            new_epoch: 1,
            ref tool_names,
        } if tool_names.iter().any(|name| name == "reload_probe")
    ));

    let summary = handle.config_summary().await;
    assert_eq!(summary["module_epoch"].as_u64(), Some(1));
    assert!(
        summary["modules"]
            .as_array()
            .expect("modules")
            .iter()
            .any(|module| module["slot"].as_str() == Some("workflow")
                && module["id"].as_str() == Some("none"))
    );
    assert!(
        summary["registered_tools"]
            .as_array()
            .expect("registered tools")
            .iter()
            .any(|tool| tool["name"].as_str() == Some("reload_probe"))
    );

    handle.shutdown().await;
}
