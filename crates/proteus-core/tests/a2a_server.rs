//! Official A2A client -> loopback JSON-RPC/SSE -> real Proteus processes ->
//! reference workflow/model/policy/patch components and canonical journals.
#[path = "a2a_server/support.rs"]
mod support;
#[path = "support/model.rs"]
mod test_model;

use a2a::*;
use futures_util::StreamExt;
use proteus_core::core::{
    JournalEntry, ModuleCatalog, TurnSettlementStatus, WorkflowReplayOptions, replay_workflow,
};
use serde_json::json;
use support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a2a_context_history_terminal_rejection_and_cold_replay() {
    let mut peer = Peer::start().await;
    let card: AgentCard = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("{}.well-known/agent-card.json", peer.url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(card.supported_interfaces[0].url, peer.url);
    assert_eq!(card.supported_interfaces[0].protocol_version, "1.0");
    assert_eq!(card.capabilities.streaming, Some(true));
    let version_error: serde_json::Value = reqwest::Client::builder().no_proxy().build().unwrap()
        .post(&peer.url).header("A2A-Version", "99.0")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "SendMessage", "params": request("wrong-version", None, None)}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(
        version_error["error"]["code"],
        error_code::VERSION_NOT_SUPPORTED
    );

    let first = peer.send(request("first-context-marker", None, None)).await;
    assert_eq!(first.status.state, TaskState::Completed, "{first:?}");
    match peer
        .client
        .subscribe_to_task(&SubscribeToTaskRequest {
            id: first.id.clone(),
            tenant: None,
        })
        .await
    {
        Err(error) => assert_eq!(error.code, error_code::UNSUPPORTED_OPERATION),
        Ok(mut stream) => assert_eq!(
            stream.next().await.unwrap().unwrap_err().code,
            error_code::UNSUPPORTED_OPERATION
        ),
    }
    let second = peer
        .send(request("followup-marker", None, Some(&first.context_id)))
        .await;
    assert_eq!(second.status.state, TaskState::Completed);
    assert_ne!(first.id, second.id);
    assert_eq!(first.context_id, second.context_id);
    let independent = peer
        .send(request("isolated-context-marker", None, None))
        .await;
    assert_ne!(first.context_id, independent.context_id);

    let rejected = peer
        .client
        .send_message(&request("must-not-execute", Some(&first.id), None))
        .await
        .unwrap_err();
    assert_eq!(rejected.code, error_code::UNSUPPORTED_OPERATION);
    let rejected_stream = peer
        .client
        .send_streaming_message(&request("stream-must-not-execute", Some(&first.id), None))
        .await;
    match rejected_stream {
        Err(error) => assert_eq!(error.code, error_code::UNSUPPORTED_OPERATION),
        Ok(mut stream) => assert_eq!(
            stream.next().await.unwrap().unwrap_err().code,
            error_code::UNSUPPORTED_OPERATION
        ),
    }
    assert_eq!(
        peer.client
            .send_message(&request(
                "wrong-context",
                Some(&first.id),
                Some(&independent.context_id)
            ))
            .await
            .unwrap_err()
            .code,
        error_code::INVALID_PARAMS
    );
    assert_eq!(
        peer.client
            .send_message(&request("unknown-task", Some("absent"), None))
            .await
            .unwrap_err()
            .code,
        error_code::TASK_NOT_FOUND
    );
    assert_eq!(
        peer.client
            .send_message(&request("unknown-context", None, Some("absent")))
            .await
            .unwrap_err()
            .code,
        error_code::INVALID_PARAMS
    );
    let trimmed = peer
        .client
        .get_task(&GetTaskRequest {
            id: second.id.clone(),
            history_length: Some(0),
            tenant: None,
        })
        .await
        .unwrap();
    assert!(trimmed.history.unwrap_or_default().is_empty());

    peer.stop().await;
    let sessions = peer.sessions();
    assert_eq!(sessions.len(), 2);
    let mut turns = 0;
    for store in sessions {
        let projection = store.load_projection().unwrap();
        let history = projection
            .history
            .iter()
            .map(|message| message.display_text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!history.contains("must-not-execute"));
        if history.contains("followup-marker") {
            assert!(history.contains("first-context-marker"));
            assert!(!history.contains("isolated-context-marker"));
        } else {
            assert!(history.contains("isolated-context-marker"));
            assert!(!history.contains("first-context-marker"));
        }
        turns += projection
            .records
            .iter()
            .filter(|record| matches!(record.entry, JournalEntry::TurnOpened(_)))
            .count();
        for record in &projection.records {
            if !matches!(record.entry, JournalEntry::TurnOpened(_)) {
                continue;
            }
            let replay = replay_workflow(
                store.session_dir(),
                &peer.config,
                &ModuleCatalog::from_config(&peer.config).unwrap(),
                WorkflowReplayOptions {
                    turn_id: record.turn_id,
                },
            )
            .await
            .unwrap();
            assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
            assert!(replay.source_journal_unchanged);
        }
    }
    assert_eq!(turns, 3, "rejected messages must never start inference");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a2a_disconnect_cancel_and_sibling_process_isolation() {
    let mut canceled_peer = Peer::start().await;
    let mut sibling = Peer::start().await;
    let long = "keep streaming while the client disconnects ".repeat(40);
    let mut stream = canceled_peer
        .client
        .send_streaming_message(&request(&long, None, None))
        .await
        .unwrap();
    let StreamResponse::Task(task) = stream.next().await.unwrap().unwrap() else {
        panic!("initial task snapshot")
    };
    drop(stream);
    wait_working(&canceled_peer, &task.id).await;
    let rejected = canceled_peer
        .client
        .send_message(&request(
            "new task in busy context",
            None,
            Some(&task.context_id),
        ))
        .await
        .unwrap_err();
    assert_eq!(rejected.code, error_code::UNSUPPORTED_OPERATION);
    let rejected = canceled_peer
        .client
        .send_message(&request(
            "live steering",
            Some(&task.id),
            Some(&task.context_id),
        ))
        .await
        .unwrap_err();
    assert_eq!(rejected.code, error_code::UNSUPPORTED_OPERATION);
    let mut reconnect = canceled_peer
        .client
        .subscribe_to_task(&SubscribeToTaskRequest {
            id: task.id.clone(),
            tenant: None,
        })
        .await
        .unwrap();
    assert!(
        matches!(reconnect.next().await.unwrap().unwrap(), StreamResponse::Task(snapshot) if snapshot.id == task.id)
    );

    let other = sibling
        .send(request("sibling survives cancellation", None, None))
        .await;
    assert_eq!(other.status.state, TaskState::Completed);
    let canceled = canceled_peer
        .client
        .cancel_task(&CancelTaskRequest {
            id: task.id.clone(),
            metadata: None,
            tenant: None,
        })
        .await
        .unwrap();
    assert_eq!(canceled.status.state, TaskState::Canceled);
    let mut terminal_seen = false;
    while let Some(event) = reconnect.next().await {
        if let StreamResponse::StatusUpdate(update) = event.unwrap() {
            terminal_seen |= update.status.state == TaskState::Canceled;
        }
    }
    assert!(terminal_seen);
    assert_eq!(
        canceled_peer
            .client
            .cancel_task(&CancelTaskRequest {
                id: task.id,
                metadata: None,
                tenant: None
            })
            .await
            .unwrap_err()
            .code,
        error_code::TASK_NOT_CANCELABLE
    );
    canceled_peer.stop().await;
    assert_eq!(
        sibling
            .send(request(
                "sibling survives peer exit",
                None,
                Some(&other.context_id)
            ))
            .await
            .status
            .state,
        TaskState::Completed
    );
    sibling.stop().await;
    let sessions = canceled_peer.sessions();
    assert_eq!(sessions.len(), 1);
    let projection = sessions[0].load_projection().unwrap();
    assert!(projection.records.iter().any(|record| matches!(&record.entry, JournalEntry::TurnSettled(settled) if settled.status == TurnSettlementStatus::Canceled)));
    assert!(
        !projection
            .history
            .iter()
            .any(|message| message.display_text().contains("live steering"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a2a_input_required_preserves_tool_approval_and_resumes_same_task() {
    let mut peer = Peer::start().await;
    let question = peer
        .send(request(
            "request_user_input choose for the test",
            None,
            None,
        ))
        .await;
    assert_eq!(
        question.status.state,
        TaskState::InputRequired,
        "{question:?}"
    );
    let input = &pending(&question)["user_inputs"][0];
    let plain_client = a2a_client::A2AClient::new(a2a_client::jsonrpc::JsonRpcTransport::new(
        reqwest::Client::builder().no_proxy().build().unwrap(),
        peer.url.clone(),
    ));
    let plain = plain_client
        .get_task(&GetTaskRequest {
            id: question.id.clone(),
            history_length: None,
            tenant: None,
        })
        .await
        .unwrap();
    assert_eq!(plain.status.state, TaskState::InputRequired);
    assert!(
        plain
            .status
            .message
            .unwrap()
            .parts
            .iter()
            .all(|part| part.as_text().is_some())
    );
    let inactive = plain_client
        .send_message(&interaction(
            &question,
            json!({"kind": "user_input", "request_id": "ignored", "response": {"answers": {}}}),
        ))
        .await
        .unwrap_err();
    assert_eq!(inactive.code, error_code::UNSUPPORTED_OPERATION);
    let request_id = input["request_id"].as_str().unwrap();
    let question_id = input["questions"][0]["id"].as_str().unwrap();
    let answered = peer
        .send(interaction(
            &question,
            json!({
                "kind": "user_input", "request_id": request_id,
                "response": {"answers": {question_id: {"answers": ["Approve"]}}}
            }),
        ))
        .await;
    assert_eq!(answered.id, question.id);
    assert_eq!(answered.status.state, TaskState::Completed, "{answered:?}");

    let patch = "apply_patch";
    let approval = peer
        .send(request(patch, None, Some(&question.context_id)))
        .await;
    assert_eq!(
        approval.status.state,
        TaskState::InputRequired,
        "{approval:?}"
    );
    assert!(!peer.workspace_file("smoke.txt").exists());
    let approval_id = pending(&approval)["approvals"][0]["approval_id"]
        .as_str()
        .unwrap();
    let invalid = peer.client.send_message(&interaction(&approval, json!({"kind": "approval", "approval_id": "different-session-request", "approved": true}))).await.unwrap_err();
    assert_eq!(invalid.code, error_code::INVALID_PARAMS);
    assert!(!peer.workspace_file("smoke.txt").exists());
    let denied = peer
        .send(interaction(
            &approval,
            json!({"kind": "approval", "approval_id": approval_id, "approved": false}),
        ))
        .await;
    assert_eq!(denied.status.state, TaskState::Completed, "{denied:?}");
    assert!(!peer.workspace_file("smoke.txt").exists());

    let approval = peer
        .send(request(patch, None, Some(&question.context_id)))
        .await;
    assert_eq!(
        approval.status.state,
        TaskState::InputRequired,
        "{approval:?}"
    );
    let approval_id = pending(&approval)["approvals"][0]["approval_id"]
        .as_str()
        .unwrap();
    let accepted = peer
        .send(interaction(
            &approval,
            json!({"kind": "approval", "approval_id": approval_id, "approved": true}),
        ))
        .await;
    assert_eq!(accepted.status.state, TaskState::Completed, "{accepted:?}");
    assert_eq!(
        std::fs::read_to_string(peer.workspace_file("smoke.txt")).unwrap(),
        "smoke\n"
    );
    peer.stop().await;
}
