//! Terminal summary failures keep the completed tool/history; no fake summary.
use super::*;
use proteus_contracts::{contracts::CancellationToken, model_standard::ModelFailureKind};
use proteus_core::core::{
    ModelResponseOutcome, ModuleCatalog, TurnSettlementStatus, WorkflowReplayOptions,
    replay_workflow,
};

#[derive(Clone, Copy)]
enum Failure {
    RetryExhausted,
    MinimalPrompt,
    Cancel,
}

async fn serve_failure(
    listener: TcpListener,
    failure: Failure,
    cancel: CancellationToken,
) -> Vec<Value> {
    let (mut socket, _) = listener.accept().await.unwrap();
    let _ = read_request(&mut socket).await;
    let FixtureReply::Json(first) = scripted_replies(false).remove(0) else {
        unreachable!()
    };
    reply(&mut socket, 200, first).await;
    let mut summaries = Vec::new();
    loop {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let request = read_request(&mut socket).await;
        let minimal = request["input"].as_array().unwrap().len() == 1;
        assert_eq!(
            text_at(request["input"].as_array().unwrap().last().unwrap()),
            COMPACTION_PROMPT
        );
        summaries.push(request);
        assert!(summaries.len() < 16, "context trimming must terminate");
        if matches!(failure, Failure::Cancel) {
            cancel.cancel();
            let mut byte = [0; 1];
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), socket.read(&mut byte))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
            break;
        }
        let overflow = matches!(failure, Failure::MinimalPrompt);
        reply(
            &mut socket,
            if overflow { 400 } else { 500 },
            json!({"error": {
                "code": if overflow {"context_length_exceeded"} else {"server_error"},
                "message": "fixture summary exhausted"
            }}),
        )
        .await;
        if (overflow && minimal) || (!overflow && summaries.len() == 2) {
            break;
        }
    }
    summaries
}

async fn reply(socket: &mut TcpStream, status: u16, body: Value) {
    let body = body.to_string();
    socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    socket.shutdown().await.unwrap();
}

async fn check(failure: Failure) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("probe.txt"), "known tool result").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = config(&format!("http://{}", listener.local_addr().unwrap()));
    config
        .module_config
        .get_mut("compactor")
        .unwrap()
        .get_mut("codex")
        .unwrap()["stream_max_retries"] = json!(1);
    config
        .module_config
        .get_mut("model")
        .unwrap()
        .get_mut("openai")
        .unwrap()["request_max_retries"] = json!(0);
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let cancel = CancellationToken::new();
    let server = tokio::spawn(serve_failure(listener, failure, cancel.clone()));
    let runtime = AgentRuntime::builder(config.clone(), root.path().to_owned())
        .with_config_path(Some(&config_path))
        .build_async()
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        runtime.run_with_cancellation(USER_TASK.into(), cancel),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    let requests = server.await.unwrap();
    match failure {
        Failure::RetryExhausted => {
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0], requests[1]);
        }
        Failure::MinimalPrompt => {
            assert!(requests.len() > 1);
            assert!(
                requests
                    .windows(2)
                    .all(|pair| pair[1]["input"].as_array().unwrap().len()
                        < pair[0]["input"].as_array().unwrap().len())
            );
            assert_eq!(
                requests.last().unwrap()["input"].as_array().unwrap().len(),
                1
            );
        }
        Failure::Cancel => assert_eq!(requests.len(), 1),
    }
    let session = runtime.session_dir().unwrap().to_owned();
    let projection = SessionStore::open(session.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(projection.history, runtime.history().await);
    assert!(projection.history.iter().any(|message| message.parts.iter().any(|part|
        matches!(&part.payload, ContentPart::ToolResult { result } if result.call_id == "call_read" && result.ok))));
    assert!(!projection.records.iter().any(|record| matches!(&record.entry,
        JournalEntry::HistoryMutated(mutation) if mutation.compaction.as_ref().is_some_and(|report| report.changed))));
    let status = if matches!(failure, Failure::Cancel) {
        TurnSettlementStatus::Canceled
    } else {
        TurnSettlementStatus::Error
    };
    assert!(projection.records.iter().any(|record| matches!(&record.entry, JournalEntry::TurnSettled(settled) if settled.status == status)));
    if !matches!(failure, Failure::Cancel) {
        let kind = if matches!(failure, Failure::MinimalPrompt) {
            ModelFailureKind::ContextWindowExceeded
        } else {
            ModelFailureKind::Other
        };
        assert!(projection.records.iter().any(|record| matches!(&record.entry,
            JournalEntry::ModelResponseRecorded(outcome) if matches!(&outcome.outcome, ModelResponseOutcome::Error { failure } if failure.kind == kind))));
    }
    drop(runtime);
    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        root.path().to_owned(),
        Some(&config_path),
        session.clone(),
    )
    .await
    .unwrap();
    assert!(cold.transcript().await.unwrap().iter().any(|item| {
        item.tool
            .as_ref()
            .is_some_and(|tool| tool.call_id == "call_read" && tool.status == "done")
    }));
    drop(cold);
    // A failed compactor has no changed checkpoint for workflow replay. Do not
    // reinterpret its internal exchange as a direct model failure or Success.
    if !matches!(failure, Failure::Cancel) {
        let error = replay_workflow(
            &session,
            &config,
            &ModuleCatalog::from_config(&config).unwrap(),
            WorkflowReplayOptions::default(),
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains(
                "compactor model exchanges require a recorded changed-compaction checkpoint"
            ),
            "{error:#}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exhausted_summary_retries_preserve_completed_tool_and_cold_history() {
    check(Failure::RetryExhausted).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_only_overflow_is_terminal_and_preserves_completed_tool() {
    check(Failure::MinimalPrompt).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_during_summary_closes_connection_and_preserves_completed_tool() {
    check(Failure::Cancel).await;
}
