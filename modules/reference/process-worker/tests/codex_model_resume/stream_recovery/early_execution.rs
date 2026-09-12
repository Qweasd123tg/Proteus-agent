//! The server deliberately withholds terminal SSE until the tool has changed
//! the workspace. A complete-then-execute implementation cannot pass this.
use super::*;
use tokio::io::AsyncReadExt;

#[derive(Clone, Copy, PartialEq)]
enum Ending {
    Complete,
    Disconnect,
    Idle,
}

async fn serve(listener: TcpListener, effect: std::path::PathBuf, ending: Ending) -> Vec<Value> {
    let (mut socket, _) = listener.accept().await.unwrap();
    let first = read_json_request(&mut socket).await;
    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    socket
        .write_all(tool_progress::completed_tool_sse().as_bytes())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        let mut progress = tokio::time::interval(Duration::from_millis(100));
        loop {
            if std::fs::read_to_string(&effect).ok().as_deref() == Some(EFFECT) {
                break;
            }
            // Shell startup is not the idle interval under test. Keep parsed
            // SSE events flowing until the effect and later commentary exist;
            // only then withhold events to trigger the intended idle timeout.
            // Comments alone do not reset the provider's parsed-event timer.
            progress.tick().await;
            socket
                .write_all(b"event: response.in_progress\ndata: {}\n\n")
                .await
                .unwrap();
        }
    })
    .await
    .expect("completed tool must execute before terminal SSE");

    // This later model item must precede the already durable tool result in
    // prompt/cold history. It forces checkpoint rebasing of the result suffix.
    let commentary = json!({"type":"message", "id":"after_early_tool", "status":"completed",
        "role":"assistant", "phase":"commentary",
        "content":[{"type":"output_text", "text":COMPLETED, "annotations":[]}]});
    socket
        .write_all(
            format!(
                "event: response.output_item.done\ndata: {}\n\n",
                json!({"output_index":2, "item":commentary})
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    if ending == Ending::Complete {
        let terminal = response(
            json!([
                {"type":"reasoning", "id":"reasoning_before_call", "summary":[], "encrypted_content":"retained-reasoning"},
                tool_response()["output"][0], commentary,
            ]),
            "early_tool_response",
        );
        socket
            .write_all(sse_body(&terminal).as_bytes())
            .await
            .unwrap();
    }
    if ending == Ending::Idle {
        let mut byte = [0u8; 1];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(4), socket.read(&mut byte))
                .await
                .expect("idle timeout must close the provider connection")
                .unwrap(),
            0
        );
    } else {
        socket.shutdown().await.unwrap();
    }
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let second = read_json_request(&mut socket).await;
    write_sse(&mut socket, &sse_body(&final_response())).await;
    vec![first, second]
}

async fn check(ending: Ending) {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = configure(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
        1,
    )
    .await;
    if ending == Ending::Idle {
        config
            .module_config
            .get_mut("model")
            .unwrap()
            .get_mut("openai")
            .unwrap()["stream_idle_timeout_ms"] = json!(1_000);
    }
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let workspace = root.path().join("workspace");
    let effect = workspace.join(EFFECT_FILE);
    let server = tokio::spawn(serve(listener, effect.clone(), ending));
    let runtime = AgentRuntime::builder(config.clone(), workspace.clone())
        .with_config_path(Some(&config_path))
        .with_approval(Arc::new(ApprovingTransport))
        .build_async()
        .await
        .unwrap();
    assert_eq!(runtime.run(PROMPT.to_owned()).await.unwrap().text, FINAL);
    let requests = server.await.unwrap();
    tool_progress::assert_completed_call_and_reasoning(&requests[1]);
    assert_retry_request(&requests[1]);
    let commentary = input_positions(&requests[1], |item| item["phase"] == "commentary")[0];
    let result = input_positions(&requests[1], |item| item["type"] == "function_call_output")[0];
    assert!(
        commentary < result,
        "results follow all completed model items"
    );
    assert_eq!(std::fs::read_to_string(&effect).unwrap(), EFFECT);

    let session = runtime.session_dir().unwrap().to_owned();
    let projection = SessionStore::open(session.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(projection.history, runtime.history().await);
    if ending == Ending::Idle {
        let failures = projection
            .records
            .iter()
            .filter_map(|record| match &record.entry {
                JournalEntry::ModelResponseRecorded(response) => match &response.outcome {
                    ModelResponseOutcome::Error { failure } => Some(failure),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            failures.len(),
            1,
            "idle must produce one typed error before retry"
        );
        assert_eq!(failures[0].kind, ModelFailureKind::StreamDisconnected);
        assert_eq!(failures[0].message, "idle timeout waiting for SSE");
    }
    let tool_started = projection.records.iter().position(|record| matches!(&record.entry, JournalEntry::ToolCallRecorded(tool) if tool.call.id == CALL_ID && tool.phase == proteus_core::core::ToolCallRecordPhase::Requested)).unwrap();
    let terminal = projection
        .records
        .iter()
        .position(|record| matches!(&record.entry, JournalEntry::ModelResponseRecorded(_)))
        .unwrap();
    assert!(
        tool_started < terminal,
        "journal proves execution while stream was open"
    );
    drop(runtime);
    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        workspace,
        Some(&config_path),
        session.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        cold.transcript()
            .await
            .unwrap()
            .iter()
            .filter_map(|item| item.tool.as_ref())
            .filter(|tool| tool.call_id == CALL_ID && tool.status == "done")
            .count(),
        1
    );
    drop(cold);
    std::fs::remove_file(&effect).unwrap();
    let replay = replay_workflow(
        &session,
        &config,
        &ModuleCatalog::from_config(&config).unwrap(),
        WorkflowReplayOptions::default(),
    )
    .await
    .unwrap();
    assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
    assert!(replay.source_journal_unchanged);
    assert!(!effect.exists(), "replay must not repeat the effect");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_tool_executes_before_terminal_sse() {
    check(Ending::Complete).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn early_tool_survives_later_disconnect_without_reexecution() {
    check(Ending::Disconnect).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn early_tool_survives_sse_idle_timeout_without_reexecution() {
    check(Ending::Idle).await;
}
