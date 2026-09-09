use proteus_contracts::contracts::CancellationToken;
use proteus_core::core::ToolCallRecordPhase;
use tokio::io::AsyncReadExt;

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_preserves_completed_reader_and_never_dispatches_queued_calls() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = config(root.path(), &listener).await;
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let directory = root.path().join("workspace");
    let approval = Arc::new(ApprovalProbe {
        deny: false,
        calls: Mutex::new(Vec::new()),
    });
    let runtime = Arc::new(
        AgentRuntime::builder(config.clone(), directory.clone())
            .with_config_path(Some(&config_path))
            .with_approval(approval.clone())
            .build_async()
            .await
            .unwrap(),
    );
    let session = runtime.session_dir().unwrap().to_owned();
    let cancel = CancellationToken::new();
    let run = tokio::spawn({
        let runtime = runtime.clone();
        let cancel = cancel.clone();
        async move {
            runtime
                .run_with_cancellation(PROMPT.to_owned(), cancel)
                .await
        }
    });
    let (mut socket, _) = open_stream(&listener, &directory, &session).await;
    release(&directory, "b");
    wait_for("second reader committed before cancellation", || {
        has_result(&session, "b")
    })
    .await;
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    wait_for("active tool receives component cancellation", || {
        directory.join("canceled-a").exists()
    })
    .await;
    let mut byte = [0; 1];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    for label in ["write", "c"] {
        assert!(!directory.join(format!("started-{label}")).exists());
    }
    assert!(approval.calls.lock().unwrap().is_empty());
    assert!(!directory.join("effects.log").exists());
    let projection = SessionStore::open(session.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(projection.history, runtime.history().await);
    assert_eq!(
        projection
            .records
            .iter()
            .filter_map(|record| match &record.entry {
                JournalEntry::TurnSettled(settled) => Some(settled.status),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [TurnSettlementStatus::Canceled]
    );
    assert!(
        projection
            .records
            .iter()
            .all(|record| !matches!(&record.entry,
        JournalEntry::ToolCallRecorded(tool) if tool.phase == ToolCallRecordPhase::Requested
            && (tool.call.id == "call_write" || tool.call.id == "call_c")))
    );
    assert!(
        projection
            .history
            .iter()
            .any(|message| message.display_text() == QUEUED)
    );
    assert_eq!(
        projection
            .history
            .iter()
            .flat_map(|message| &message.parts)
            .filter_map(|part| match &part.payload {
                ContentPart::ToolResult { result } if result.ok => Some(result.call_id.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        ["call_b"]
    );
    drop(runtime);
    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config,
        directory,
        Some(&config_path),
        session,
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    assert_eq!(
        transcript
            .iter()
            .filter_map(|item| item.tool.as_ref())
            .filter(|tool| tool.call_id == "call_b" && tool.status == "done")
            .count(),
        1
    );
    assert!(
        transcript
            .iter()
            .filter_map(|item| item.tool.as_ref())
            .all(|tool| tool.call_id == "call_b" || tool.status != "done")
    );
}
