//! Completed calls survive a failed sample; partial argument streams do not.
use super::*;

pub(super) fn completed_tool_sse() -> String {
    let reasoning = json!({"type": "reasoning", "id": "reasoning_before_call",
        "summary": [], "encrypted_content": "retained-reasoning"});
    let tool = tool_response()["output"][0].clone();
    let partial = json!({"type": "function_call", "id": "unfinished_tool_item",
        "call_id": "unfinished_tool_call", "name": "shell", "arguments": ""});
    [
        format!(
            "event: response.output_item.done\ndata: {}\n\n",
            json!({"output_index": 0, "item": reasoning})
        ),
        format!(
            "event: response.output_item.done\ndata: {}\n\n",
            json!({"output_index": 1, "item": tool})
        ),
        // Repeated delivery of the same item must not duplicate execution.
        format!(
            "event: response.output_item.done\ndata: {}\n\n",
            json!({"output_index": 1, "item": tool})
        ),
        format!(
            "event: response.output_item.added\ndata: {}\n\n",
            json!({"output_index": 2, "item": partial})
        ),
        format!(
            "event: response.function_call_arguments.delta\ndata: {}\n\n",
            json!({
            "output_index": 2, "item_id": "unfinished_tool_item", "delta": tool["arguments"]})
        ),
    ]
    .concat()
}

pub(super) fn assert_completed_call_and_reasoning(request: &Value) {
    let input = request["input"].as_array().unwrap();
    let call = input
        .iter()
        .position(|item| item["type"] == "function_call" && item["call_id"] == CALL_ID)
        .unwrap();
    let result = input
        .iter()
        .position(|item| item["type"] == "function_call_output" && item["call_id"] == CALL_ID)
        .unwrap();
    let reasoning = input
        .iter()
        .position(|item| {
            item["type"] == "reasoning" && item["encrypted_content"] == "retained-reasoning"
        })
        .unwrap();
    assert!(reasoning < call && call < result);
    assert_eq!(
        input[call]["arguments"],
        tool_response()["output"][0]["arguments"]
    );
    assert!(
        !input
            .iter()
            .any(|item| item["call_id"] == "unfinished_tool_call")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_tool_is_drained_when_stream_retry_is_disabled() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = configure(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
        0,
    )
    .await;
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_json_request(&mut socket).await;
        write_sse(&mut socket, &completed_tool_sse()).await;
    });
    let runtime = AgentRuntime::builder(config.clone(), root.path().join("workspace"))
        .with_config_path(Some(&config_path))
        .with_approval(Arc::new(ApprovingTransport))
        .build_async()
        .await
        .unwrap();
    let error = runtime.run(PROMPT.to_owned()).await.unwrap_err();
    assert!(
        format!("{error:#}").contains("stream ended without a terminal event"),
        "{error:#}"
    );
    server.await.unwrap();
    let effect = root.path().join("workspace").join(EFFECT_FILE);
    assert_eq!(std::fs::read_to_string(&effect).unwrap(), EFFECT);

    let session_dir = runtime.session_dir().unwrap().to_owned();
    let projection = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(projection.history, runtime.history().await);
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
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].kind, ModelFailureKind::StreamDisconnected);
    let retained = failures[0]
        .completed_messages
        .iter()
        .find(|message| {
            message.parts.iter().any(|part| {
        matches!(&part.payload, ContentPart::ToolCall { call } if call.id == CALL_ID)
    })
        })
        .expect("failure retains the call with its canonical identity");
    assert!(projection.history.contains(retained));
    assert_eq!(projection.history.iter().flat_map(|message| &message.parts).filter(|part| {
        matches!(&part.payload, ContentPart::ToolResult { result } if result.call_id == CALL_ID)
    }).count(), 1);
    assert!(!projection.history.iter().flat_map(|message| &message.parts).any(|part| {
        matches!(&part.payload, ContentPart::ToolCall { call } if call.id == "unfinished_tool_call")
    }));
    assert_eq!(
        projection
            .records
            .iter()
            .filter_map(|record| match &record.entry {
                JournalEntry::TurnSettled(settled) => Some(settled.status),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [TurnSettlementStatus::Error]
    );
    drop(runtime);
    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config.clone(),
        root.path().join("workspace"),
        Some(&config_path),
        session_dir.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        cold.transcript()
            .await
            .unwrap()
            .iter()
            .filter_map(|item| item.tool.as_ref())
            .filter(|tool| tool.call_id == CALL_ID
                && tool.status == "done"
                && tool
                    .result
                    .as_ref()
                    .is_some_and(|text| text.contains("committed")))
            .count(),
        1
    );
    drop(cold);
    std::fs::remove_file(&effect).unwrap();
    let replay = replay_workflow(
        &session_dir,
        &config,
        &ModuleCatalog::from_config(&config).unwrap(),
        WorkflowReplayOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(replay.replay.status, TurnSettlementStatus::Error);
    assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
    assert!(replay.source_journal_unchanged);
    assert!(!effect.exists());
}
