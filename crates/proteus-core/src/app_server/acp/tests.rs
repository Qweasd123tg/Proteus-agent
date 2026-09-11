use super::*;
use crate::domain::{
    Event, EventContext, EventEnvelope, new_message_id, new_session_id, new_thread_id,
};

#[tokio::test]
async fn protocol_prompt_stays_reserved_after_runtime_has_settled() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::test_model::config();
    crate::test_support::select_test_modules(&mut config, "coding.single_loop");
    config.tools.enabled.clear();
    config.event_log.path = dir.path().join("events.jsonl");
    let server = AgentAppServer::launch_with_module_catalog(
        config,
        dir.path().to_path_buf(),
        None,
        crate::test_support::module_catalog(),
    )
    .await
    .unwrap();
    let session = Session {
        server,
        active: Arc::new(StdMutex::new(None)),
    };
    let lease = session.reserve().unwrap();
    session
        .server
        .send_user_message_with_cancellation("hello".into(), lease.cancellation.clone())
        .await
        .unwrap();
    assert!(session.server.running_run_ids().await.is_empty());
    // ACP may still be draining notifications even though the runtime is idle.
    assert!(session.reserve().is_err());
    drop(lease);
    assert!(session.reserve().is_ok());
    session.server.shutdown().await;
}

#[test]
fn canonical_completion_does_not_repeat_streamed_unicode_or_peer_text() {
    let session = new_session_id();
    let thread = new_thread_id();
    let mut projection = projection::Projection::new(session);
    let context = EventContext::new(session, thread, None);
    let event = |event| EventEnvelope::new(context.clone(), 0, event);
    projection
        .event(event(Event::TurnStarted {
            session_id: session,
            thread_id: thread,
            turn_id: crate::domain::new_turn_id(),
        }))
        .unwrap();
    let id = new_message_id();
    let first = projection
        .event(event(Event::AssistantTextDelta {
            message_id: id,
            phase: None,
            offset: 0,
            text: "Привет".into(),
        }))
        .unwrap();
    assert_eq!(first.len(), 1);
    assert!(
        projection
            .event(event(Event::AssistantTextDelta {
                message_id: id,
                phase: None,
                offset: 0,
                text: "При".into(),
            }))
            .unwrap()
            .is_empty()
    );
    let tail = projection
        .event(event(Event::AssistantMessageCompleted {
            message_id: id,
            phase: None,
            text: "Привет!".into(),
        }))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&tail[0]).unwrap()["content"]["text"],
        "!"
    );
    let mut peer = event(Event::AssistantMessageCompleted {
        message_id: new_message_id(),
        phase: None,
        text: "peer".into(),
    });
    peer.thread_id = new_thread_id();
    assert!(projection.event(peer).unwrap().is_empty());
    assert!(projection.fallback_output("Привет!".into()).is_none());
    let next = projection
        .event(event(Event::AssistantMessageCompleted {
            message_id: new_message_id(),
            phase: None,
            text: "Final".into(),
        }))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&next[0]).unwrap()["content"]["text"],
        "\n\nFinal"
    );
    assert!(
        projection
            .event(event(Event::AssistantTextDelta {
                message_id: id,
                phase: None,
                offset: 1,
                text: "bad".into(),
            }))
            .is_err()
    );
}

#[test]
fn unsupported_inputs_and_duplicate_mcp_names_are_rejected() {
    let image = serde_json::from_value(serde_json::json!({
        "type":"image", "data":"aGVsbG8=", "mimeType":"image/png"
    }))
    .unwrap();
    assert!(input::prompt_text(vec![image]).is_err());
    assert!(input::prompt_text(vec![]).is_err());
    let link = serde_json::from_value(serde_json::json!({
        "type":"resource_link", "name":"file", "uri":"file:///tmp/test.txt"
    }))
    .unwrap();
    assert!(
        input::prompt_text(vec![link])
            .unwrap()
            .contains("file:///tmp/test.txt")
    );
    let server: McpServer = serde_json::from_value(serde_json::json!({
        "name":"editor", "command":"/bin/test", "args":[], "env":[{"name":"KEY","value":"value"}]
    }))
    .unwrap();
    let config = input::session_config(AppConfig::default(), vec![server.clone()]).unwrap();
    assert_eq!(config.tools.mcp_servers[0].environment.env["KEY"], "value");
    assert!(input::session_config(config, vec![server]).is_err());
}
