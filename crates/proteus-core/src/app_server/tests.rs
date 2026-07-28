use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use coding_workflow::CodingPlanExecuteReviewWorkflow;
use context_pack::SimpleContextBuilderPlugin;
use proteus_contracts::{
    abi_stable::sabi_trait::TD_Opaque,
    plugin::{PluginContextBuilder_TO, PluginWorkflow_TO},
};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};

use super::*;
use crate::{
    contracts::{UserInputQuestion, UserInputQuestionOption, UserInputRequest},
    core::{PendingUserInput, SessionStore},
    domain::{Event, ToolCall, ToolResult, new_session_id},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
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
}

fn test_user_input_request(request_id: &str) -> UserInputRequest {
    UserInputRequest::new(request_id.to_owned(), PathBuf::from("."), Vec::new())
}

/// Регистрирует pending user input так же, как это делает forwarder:
/// запись в map + watcher, владеющий responder-ом.
async fn register_test_user_input(
    pending_user_inputs: &user_inputs::PendingUserInputResponders,
    events: &broadcast::Sender<AppServerEvent>,
    request_id: &str,
    responder: oneshot::Sender<UserInputResponse>,
) {
    user_inputs::register_pending_user_input(
        pending_user_inputs,
        events,
        test_user_input_request(request_id),
        responder,
    )
    .await;
}

#[tokio::test]
async fn user_input_forwarder_waits_without_timeout_when_timeout_is_zero() {
    let (user_input_tx, user_input_rx) = mpsc::channel(1);
    let (events, _) = broadcast::channel(8);
    let mut event_rx = events.subscribe();
    let pending_user_inputs = Arc::new(Mutex::new(HashMap::new()));
    user_inputs::spawn_user_input_forwarder(
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
async fn shutdown_resolves_pending_user_inputs() {
    let (events, _) = broadcast::channel(8);
    let pending_user_inputs: user_inputs::PendingUserInputResponders =
        Arc::new(Mutex::new(HashMap::new()));
    let (responder, response_rx) = oneshot::channel();
    let request_id = "input-1".to_owned();
    register_test_user_input(&pending_user_inputs, &events, &request_id, responder).await;
    // Подписка после регистрации: первым событием интересует resolved.
    let mut event_rx = events.subscribe();

    user_inputs::resolve_pending_user_inputs_empty(pending_user_inputs.clone()).await;

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

/// Ключевое поведение очереди user inputs: если
/// запросивший (tool при отмене turn-а) дропает свой future, watcher убирает
/// осиротевшую запись и сообщает клиентам resolved — без blanket-resolve
/// остальных pending user inputs.
#[tokio::test]
async fn dropped_requester_removes_pending_user_input_and_resolves_it() {
    let (events, _) = broadcast::channel(8);
    let pending_user_inputs: user_inputs::PendingUserInputResponders =
        Arc::new(Mutex::new(HashMap::new()));

    let (cancelled_responder, cancelled_rx) = oneshot::channel();
    let (survivor_responder, mut survivor_rx) = oneshot::channel();
    register_test_user_input(
        &pending_user_inputs,
        &events,
        "input-cancelled",
        cancelled_responder,
    )
    .await;
    register_test_user_input(
        &pending_user_inputs,
        &events,
        "input-survivor",
        survivor_responder,
    )
    .await;
    let mut event_rx = events.subscribe();

    // Отмена turn-а: orchestrator дропает tool future -> receiver закрыт.
    drop(cancelled_rx);

    let resolved_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("user input resolved event should arrive")
        .expect("event stream should stay open");
    assert!(matches!(
        resolved_event,
        AppServerEvent::UserInputResolved { request_id: id } if id == "input-cancelled"
    ));

    // Второй pending user input не задет.
    let pending = pending_user_inputs.lock().await;
    assert!(!pending.contains_key("input-cancelled"));
    assert!(pending.contains_key("input-survivor"));
    drop(pending);
    assert!(survivor_rx.try_recv().is_err());
}

/// Ответ клиента резолвит именно свой запрос; событие эмитит watcher.
#[tokio::test]
async fn resolve_pending_user_input_forwards_response_and_emits_event() {
    let (events, _) = broadcast::channel(8);
    let pending_user_inputs: user_inputs::PendingUserInputResponders =
        Arc::new(Mutex::new(HashMap::new()));
    let (responder, response_rx) = oneshot::channel();
    let request_id = "input-resolve".to_owned();
    register_test_user_input(&pending_user_inputs, &events, &request_id, responder).await;
    let mut event_rx = events.subscribe();

    user_inputs::resolve_pending_user_input(
        &pending_user_inputs,
        &request_id,
        UserInputResponse::empty(),
    )
    .await
    .expect("resolve should succeed");

    let response = tokio::time::timeout(Duration::from_secs(1), response_rx)
        .await
        .expect("user input response should not hang")
        .expect("user input responder should receive answer");
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

    let unknown = user_inputs::resolve_pending_user_input(
        &pending_user_inputs,
        &request_id,
        UserInputResponse::empty(),
    )
    .await;
    assert!(unknown.is_err(), "second resolve must report unknown id");
}

#[tokio::test]
async fn zero_timeout_pending_user_input_resolves_on_shutdown() {
    let (user_input_tx, user_input_rx) = mpsc::channel(1);
    let (events, _) = broadcast::channel(8);
    let mut event_rx = events.subscribe();
    let pending_user_inputs = Arc::new(Mutex::new(HashMap::new()));
    user_inputs::spawn_user_input_forwarder(
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

    user_inputs::resolve_pending_user_inputs_empty(pending_user_inputs.clone()).await;

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
    config.modules.renderer = "text".to_owned();
    config.modules.patch = "null".to_owned();

    let handle = AgentAppServer::launch_with_module_catalog(
        config,
        cwd.path().to_path_buf(),
        None,
        test_catalog(),
    )
    .await
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
    config.modules.renderer = "text".to_owned();
    config.modules.patch = "null".to_owned();

    let handle = AgentAppServer::launch_with_module_catalog(
        config,
        cwd.path().to_path_buf(),
        None,
        test_catalog(),
    )
    .await
    .expect("app server");

    handle
        .send_user_message("restore this chat".to_owned())
        .await
        .expect("turn output");

    let transcript = handle.transcript().await.expect("transcript");
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

#[test]
fn transcript_projects_tool_calls_as_restorable_tool_cards() {
    let call = ToolCall::new(
        "call-1",
        "read_file",
        serde_json::json!({"path": "src/lib.rs"}),
    );
    let result = ToolResult::ok("call-1".to_owned(), "line 1\nline 2");
    let transcript = transcript_messages(&[
        CanonicalMessage::new(MessageRole::Assistant, vec![ContentPart::ToolCall { call }]),
        CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }]),
    ]);

    assert_eq!(transcript.len(), 1);
    let tool = transcript[0].tool.as_ref().expect("tool transcript");
    assert_eq!(tool.call_id, "call-1");
    assert_eq!(tool.name, "read_file");
    assert_eq!(tool.args, serde_json::json!({"path": "src/lib.rs"}));
    assert_eq!(tool.status, "done");
    assert_eq!(tool.result.as_deref(), Some("line 1\nline 2"));
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
    .await
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
    let saved_store =
        SessionStore::new(config_dir.path(), cwd.path(), saved_session_id).expect("session store");
    saved_store
        .append_history(
            crate::domain::new_thread_id(),
            None,
            &[CanonicalMessage::text(
                MessageRole::User,
                "restore saved chat",
            )],
        )
        .await
        .expect("append saved messages");

    let empty_store = SessionStore::new(config_dir.path(), cwd.path(), new_session_id())
        .expect("empty session store");
    let empty_thread = crate::domain::new_thread_id();
    empty_store
        .append_history(
            empty_thread,
            None,
            &[CanonicalMessage::text(MessageRole::User, "temporary")],
        )
        .await
        .expect("materialize empty session");
    empty_store
        .clear_history(empty_thread)
        .await
        .expect("clear empty session");

    let handle = AgentAppServer::launch_or_resume_latest(
        AppConfig::default(),
        cwd.path().to_path_buf(),
        Some(&config_path),
    )
    .await
    .expect("app server");

    assert_eq!(
        handle.runtime.session_dir(),
        Some(saved_store.session_dir())
    );
    assert_eq!(handle.runtime.history().await.len(), 1);
    assert_eq!(
        handle.transcript().await.expect("transcript")[0].text,
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
active_provider = "fake"

[providers.fake]

[tools]
enabled = []
"#,
    )
    .expect("initial config");
    let config = AppConfig::load(Some(&config_path))
        .await
        .expect("load initial config");
    let handle = AgentAppServer::launch(config, cwd.path().to_path_buf(), Some(&config_path))
        .await
        .expect("app server");
    let mut event_rx = handle.subscribe();

    std::fs::write(
        &config_path,
        r#"
active_provider = "fake"

[providers.fake]

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

#[tokio::test]
async fn runtime_forwarder_lag_emits_typed_event_stream_lagged() {
    use crate::{
        contracts::EventSink,
        domain::{EventContext, new_thread_id, new_turn_id},
    };

    // Ring на 1 слот: подписываемся до отправки, затем переполняем канал —
    // старые события безвозвратно выброшены, и форвардер обязан сообщить об
    // этом типизированным EventStreamLagged (не Error: «ход упал» и «клиент
    // отстал и должен пересинхронизироваться» — разные контракты).
    let core_broadcast = Arc::new(BroadcastEventSink::new(1));
    let lagging_rx = core_broadcast.subscribe();
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    for seq in 0..3_u64 {
        core_broadcast
            .append(EventEnvelope::new(
                EventContext::new(session_id, thread_id, Some(new_turn_id())),
                seq,
                Event::TaskReceived {
                    task: crate::domain::AgentTask::new(format!("event {seq}"), PathBuf::from(".")),
                },
            ))
            .await
            .expect("append to broadcast sink");
    }

    let (events, mut events_rx) = broadcast::channel(16);
    spawn_runtime_event_forwarder_with_receiver(
        lagging_rx,
        events,
        Arc::new(Mutex::new(TurnProgress::default())),
    );

    let first = tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .expect("forwarder output before timeout")
        .expect("forwarder event");
    match first {
        AppServerEvent::EventStreamLagged { count } => assert!(count > 0),
        other => panic!("expected EventStreamLagged, got {other:?}"),
    }

    // После уведомления форвардер продолжает доставлять то, что уцелело.
    let second = tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .expect("runtime event before timeout")
        .expect("runtime event");
    assert!(matches!(second, AppServerEvent::Runtime { .. }));
}
