//! A host export override still constrains a tool with a longer declared timeout.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_export_timeout_constrains_terminal_and_replays_recorded_error() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = terminal_config(&format!("http://{}", listener.local_addr().unwrap()));
    let mut value = serde_json::to_value(config).unwrap();
    value["components"]["fixture"]["exports"]["tool"]["reference.tools"]["timeout_ms"] =
        json!(1000);
    let config: AppConfig = serde_json::from_value(value).unwrap();
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let server =
        tokio::spawn(async move {
            let (mut socket, _) = accept(&listener).await;
            respond(&mut socket, json!([call("call_exec", "exec_command", json!({
            "cmd": "sleep 20", "yield_time_ms": 30000, "with_escalated_permissions": true
        }))])).await;
            let (mut socket, request) = accept(&listener).await;
            assert!(
                tool_text(&request, "call_exec").contains("timed out"),
                "{request}"
            );
            respond(
                &mut socket,
                json!([message("final_answer", &["Export deadline observed."])]),
            )
            .await;
        });
    let runtime = AgentRuntime::builder(config.clone(), root.path().to_owned())
        .with_config_path(Some(&config_path))
        .build_async()
        .await
        .unwrap();
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        runtime.run("Check terminal deadline.".into()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(output.text, "Export deadline observed.");
    server.await.unwrap();
    let session = runtime.session_dir().unwrap().to_owned();
    let projection = SessionStore::open(session.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    let results = history_results(&projection.history);
    assert_eq!(results.len(), 1);
    assert!(!results[0].ok);
    assert!(results[0].error.as_ref().unwrap().contains("timed out"));
    drop(runtime);
    let replay = replay_workflow(
        &session,
        &config,
        &ModuleCatalog::from_config(&config).unwrap(),
        WorkflowReplayOptions::default(),
    )
    .await
    .unwrap();
    assert!(replay.comparison.matched, "{replay:#?}");
    assert!(replay.source_journal_unchanged);
}
