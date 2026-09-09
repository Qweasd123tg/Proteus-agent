use super::*;
use proteus_contracts::contracts::CancellationToken;
use proteus_core::core::ToolCallRecordPhase;

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_long_poll_stops_process_and_preserves_completed_launch_in_cold_history() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = terminal_config(&format!("http://{}", listener.local_addr().unwrap()));
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let runtime = std::sync::Arc::new(
        AgentRuntime::builder(config.clone(), root.path().to_owned())
            .with_config_path(Some(&config_path))
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
                .run_with_cancellation("Check terminal.".into(), cancel)
                .await
        }
    });
    let (mut socket, _) = accept(&listener).await;
    respond(&mut socket, json!([call("call_exec", "exec_command", json!({
        "cmd": "sleep 20 & echo $! > child.pid; wait", "yield_time_ms": 250, "with_escalated_permissions": true
    }))])).await;
    let (mut socket, request) = accept(&listener).await;
    let id = session_id(&request);
    wait_for_file(&root.path().join("child.pid")).await;
    respond(
        &mut socket,
        json!([call(
            "call_poll",
            "write_stdin",
            json!({"session_id": id, "yield_time_ms": 300000})
        )]),
    )
    .await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let projection = SessionStore::open(session.clone())
            .unwrap()
            .load_projection()
            .unwrap();
        if projection.records.iter().any(|record| matches!(&record.entry,
            JournalEntry::ToolCallRecorded(tool) if tool.call.id == "call_poll" && tool.phase == ToolCallRecordPhase::Requested)) { break; }
        assert!(
            tokio::time::Instant::now() < deadline,
            "poll never dispatched"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The process tool must be inside its wait, not merely queued for dispatch.
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let pid = std::fs::read_to_string(root.path().join("child.pid")).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim()));
        if !stat.is_ok_and(|stat| !stat.split_once(") ").unwrap().1.starts_with(['Z', 'X'])) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "canceled process descendant survived"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let projection = SessionStore::open(session.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(projection.history, runtime.history().await);
    assert!(
        history_results(&projection.history)
            .iter()
            .any(|result| result.call_id == "call_exec" && result.ok)
    );
    assert!(
        projection
            .records
            .iter()
            .any(|record| matches!(&record.entry,
        JournalEntry::TurnSettled(settled) if settled.status == TurnSettlementStatus::Canceled))
    );
    drop(runtime);
    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config,
        root.path().to_owned(),
        Some(&config_path),
        session,
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    assert!(
        transcript
            .iter()
            .filter_map(|item| item.tool.as_ref())
            .any(|tool| tool.call_id == "call_exec" && tool.status == "done")
    );
    assert!(
        !transcript
            .iter()
            .filter_map(|item| item.tool.as_ref())
            .any(|tool| tool.call_id == "call_poll" && tool.status == "done")
    );
}
