#[path = "acp_server/fixture.rs"]
mod fixture;
#[path = "support/model.rs"]
mod test_model;

use fixture::{Client, text};
use proteus_core::core::{JournalEntry, ModuleCatalog, TurnSettlementStatus, replay_workflow};
use serde_json::json;

#[tokio::test]
async fn stdio_sessions_stream_once_preserve_history_and_replay() {
    let mut client = Client::launch(1).await;
    client
        .request(0, "session/new", json!({"cwd":client.cwd,"mcpServers":[]}))
        .await;
    assert_eq!(client.response(0).await.0["error"]["code"], -32602);
    client.initialize().await;
    let first = client.new_session(2).await;
    let second = client.new_session(3).await;
    assert_ne!(first, second);
    client.prompt(4, &first, "Привет ACP").await;
    let (response, updates) = client.response(4).await;
    assert_eq!(response["result"]["stopReason"], "end_turn", "{response}");
    assert!(updates.len() > 1, "expected streaming chunks");
    assert!(updates.iter().all(|u| u["params"]["sessionId"] == first));
    let rendered = text(&updates);
    assert_eq!(
        rendered.matches("Fake final answer.").count(),
        1,
        "{rendered}"
    );
    assert!(rendered.contains("Привет ACP"));
    client.prompt(5, &second, "isolated").await;
    let (response, _) = client.response(5).await;
    assert_eq!(response["result"]["stopReason"], "end_turn");
    client.close().await;
    let first_store = client.store(&first);
    let history = first_store.load_messages().unwrap();
    assert_eq!(history.last().unwrap().display_text(), rendered);
    assert!(
        !client
            .store(&second)
            .load_messages()
            .unwrap()
            .iter()
            .any(|m| m.display_text().contains("Привет ACP"))
    );
    let replay = replay_workflow(
        first_store.journal_path(),
        &client.config,
        &ModuleCatalog::from_config(&client.config).unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    assert!(replay.comparison.matched, "{:?}", replay.comparison);
    assert!(replay.source_journal_unchanged);
}

#[tokio::test]
async fn approval_allow_deny_and_invalid_option_use_the_runtime_policy_path() {
    for option in ["allow_once", "reject_once", "invalid-option"] {
        let mut client = Client::launch(1).await;
        client.initialize().await;
        let session = client.new_session(2).await;
        client.prompt(3, &session, "apply_patch").await;
        let request = loop {
            let value = client.read().await;
            if value["method"] == "session/request_permission" {
                break value;
            }
            assert!(value.get("error").is_none(), "{value}");
        };
        assert_eq!(request["params"]["sessionId"], session);
        assert_eq!(request["params"]["options"].as_array().unwrap().len(), 2);
        assert!(!client.cwd.join("smoke.txt").exists());
        client
            .write(json!({"jsonrpc":"2.0","id":request["id"],"result":{
                "outcome":{"outcome":"selected","optionId":option}
            }}))
            .await;
        let (response, updates) = client.response(3).await;
        assert_eq!(response["result"]["stopReason"], "end_turn", "{response}");
        assert!(
            updates
                .iter()
                .any(|u| u["params"]["update"]["sessionUpdate"] == "tool_call_update")
        );
        assert_eq!(
            client.cwd.join("smoke.txt").exists(),
            option == "allow_once"
        );
        client.close().await;
        let replay = replay_workflow(
            client.store(&session).journal_path(),
            &client.config,
            &ModuleCatalog::from_config(&client.config).unwrap(),
            Default::default(),
        )
        .await
        .unwrap();
        assert!(
            replay.comparison.matched,
            "{option}: {:?}",
            replay.comparison
        );
    }
}

#[tokio::test]
async fn cancel_unanswered_permission_settles_before_response_and_allows_next_prompt() {
    let mut client = Client::launch(1).await;
    client.initialize().await;
    let session = client.new_session(2).await;
    let other = client.new_session(20).await;
    client.prompt(3, &session, "apply_patch").await;
    loop {
        let event = client.read().await;
        if event["method"] == "session/request_permission" {
            break;
        }
        assert!(event.get("error").is_none(), "{event}");
    }
    client.prompt(4, &session, "must not queue").await;
    assert_eq!(client.response(4).await.0["error"]["code"], -32602);
    // Waiting on this session's permission must not block another session.
    client.prompt(21, &other, "independent prompt").await;
    let (response, updates) = client.response(21).await;
    assert_eq!(response["result"]["stopReason"], "end_turn");
    assert!(updates.iter().all(|u| u["params"]["sessionId"] == other));
    client.cancel(&session).await;
    let (response, _) = client.response(3).await;
    assert_eq!(response["result"]["stopReason"], "cancelled", "{response}");
    assert!(!client.cwd.join("smoke.txt").exists());
    client.prompt(5, &session, "after cancel").await;
    assert_eq!(
        client.response(5).await.0["result"]["stopReason"],
        "end_turn"
    );
    client.close().await;
    let store = client.store(&session);
    let records = store.load_records().unwrap();
    assert!(records.iter().any(|r| matches!(&r.entry, JournalEntry::TurnSettled(t) if t.status == TurnSettlementStatus::Canceled)));
    assert!(
        !store
            .load_messages()
            .unwrap()
            .iter()
            .any(|m| m.display_text().contains("must not queue"))
    );
}

#[tokio::test]
async fn eof_during_stream_cancels_and_preserves_cold_history() {
    let mut client = Client::launch(100).await;
    client.initialize().await;
    let session = client.new_session(2).await;
    client.prompt(3, &session, "cancel on EOF").await;
    loop {
        let event = client.read().await;
        if event["params"]["update"]["sessionUpdate"] == "agent_message_chunk" {
            break;
        }
        assert!(event.get("error").is_none(), "{event}");
    }
    client.close().await;
    let store = client.store(&session);
    assert!(store.load_records().unwrap().iter().any(|r|
        matches!(&r.entry, JournalEntry::TurnSettled(t) if t.status == TurnSettlementStatus::Canceled)));
    assert!(
        store
            .load_messages()
            .unwrap()
            .iter()
            .any(|m| m.display_text().contains("cancel on EOF"))
    );
}

#[tokio::test]
async fn unsupported_methods_content_and_modes_fail_without_inference() {
    let mut client = Client::launch(1).await;
    client.initialize().await;
    client
        .request(2, "session/new", json!({"cwd":"relative","mcpServers":[]}))
        .await;
    assert_eq!(client.response(2).await.0["error"]["code"], -32602);
    let session = client.new_session(3).await;
    client
        .request(
            4,
            "session/load",
            json!({"sessionId":session,"cwd":client.cwd,"mcpServers":[]}),
        )
        .await;
    assert_eq!(client.response(4).await.0["error"]["code"], -32601);
    client
        .request(
            5,
            "session/prompt",
            json!({"sessionId":session,"prompt":[{
                "type":"image","data":"aGVsbG8=","mimeType":"image/png"
            }]}),
        )
        .await;
    assert_eq!(client.response(5).await.0["error"]["code"], -32602);
    client
        .request(
            6,
            "session/set_mode",
            json!({"sessionId":session,"modeId":"unknown"}),
        )
        .await;
    assert_eq!(client.response(6).await.0["error"]["code"], -32602);
    client
        .request(
            7,
            "session/set_mode",
            json!({"sessionId":session,"modeId":"plan"}),
        )
        .await;
    let (response, updates) = client.response(7).await;
    assert!(response.get("result").is_some());
    assert_eq!(updates[0]["params"]["update"]["currentModeId"], "plan");
    client.close().await;
    assert!(
        proteus_core::core::list_workspace_session_summaries(client.dir.path(), &client.cwd)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn editor_stdio_mcp_is_discovered_in_only_its_session() {
    let mut client = Client::launch(1).await;
    client.initialize().await;
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mcp/echo_server.sh");
    client
        .request(
            2,
            "session/new",
            json!({"cwd":client.cwd,"mcpServers":[{
                "name":"editor_echo","command":"/bin/sh","args":[script],"env":[]
            }]}),
        )
        .await;
    let response = client.response(2).await.0;
    let session = response["result"]["sessionId"]
        .as_str()
        .expect(&response.to_string())
        .to_owned();
    client.prompt(3, &session, "MCP discovery").await;
    assert_eq!(
        client.response(3).await.0["result"]["stopReason"],
        "end_turn"
    );
    let other = client.new_session(4).await;
    client.prompt(5, &other, "no editor MCP").await;
    assert_eq!(
        client.response(5).await.0["result"]["stopReason"],
        "end_turn"
    );
    client.close().await;
    for (id, expected) in [(&session, true), (&other, false)] {
        let records = client.store(id).load_records().unwrap();
        let model_request = records
            .iter()
            .find_map(|r| match &r.entry {
                JournalEntry::ModelRequestRecorded(r) => Some(&r.request),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            model_request
                .tools
                .iter()
                .any(|t| t.name == "editor_echo__echo"),
            expected
        );
    }
}

#[tokio::test]
async fn structured_question_without_ui_is_resolved_instead_of_hanging() {
    let mut client = Client::launch(1).await;
    client.initialize().await;
    let session = client.new_session(2).await;
    client.prompt(3, &session, "request_user_input").await;
    let (response, updates) = client.response(3).await;
    assert_eq!(response["result"]["stopReason"], "end_turn", "{response}");
    assert!(text(&updates).contains("cannot answer structured questions"));
    client.close().await;
}

#[tokio::test]
async fn runtime_timeout_is_an_error_with_its_own_cold_settlement() {
    let mut client = Client::launch_with_timeout(500, 200).await;
    client.initialize().await;
    let session = client.new_session(2).await;
    client.prompt(3, &session, "timeout").await;
    let (response, _) = client.response(3).await;
    assert_eq!(response["error"]["code"], -32603, "{response}");
    client.close().await;
    let store = client.store(&session);
    let statuses = store
        .load_records()
        .unwrap()
        .into_iter()
        .filter_map(|r| match r.entry {
            JournalEntry::TurnSettled(t) => Some(t.status),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(statuses, [TurnSettlementStatus::Timeout]);
    assert!(
        store
            .load_messages()
            .unwrap()
            .iter()
            .any(|m| m.display_text() == "timeout")
    );
}
