use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summary_can_exceed_thirty_seconds_within_the_workflow_budget() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = config(&format!("http://{}", listener.local_addr().unwrap()));
    config.runtime.model_timeout_ms = 40_000;
    config.runtime.workflow_timeout_ms = 60_000;
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    std::fs::write(root.path().join("probe.txt"), "fixture file contents").unwrap();
    let mut replies = scripted_replies(false);
    let FixtureReply::Json(body) = replies.remove(1) else {
        unreachable!()
    };
    replies.insert(
        1,
        FixtureReply::Delayed {
            delay: Duration::from_secs(31),
            body,
        },
    );
    let server = tokio::spawn(serve(listener, replies));
    let runtime = AgentRuntime::builder(config, root.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .build_async()
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(65), runtime.run(USER_TASK.to_owned()))
        .await
        .unwrap();
    assert_eq!(
        result
            .expect("summary inherits the whole workflow budget")
            .text,
        FINAL_TEXT
    );
    assert_eq!(server.await.unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn large_current_input_is_compacted_and_survives_cold_resume() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = config(&format!("http://{}", listener.local_addr().unwrap()));
    let prompt = format!("BEGIN:{}:END", "abcdefghij".repeat(10_000));
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let server = tokio::spawn(serve(
        listener,
        vec![
            FixtureReply::Json(completed_response(
                "summary",
                vec![assistant("final_answer", SUMMARY_TEXT)],
                None,
            )),
            FixtureReply::Json(completed_response(
                "final",
                vec![assistant("final_answer", FINAL_TEXT)],
                None,
            )),
            FixtureReply::Json(completed_response(
                "continued",
                vec![assistant("final_answer", "Продолжено.")],
                None,
            )),
        ],
    ));
    let thread_id = proteus_contracts::domain::new_thread_id();
    let runtime = AgentRuntime::builder(config.clone(), root.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_session_ids(new_session_id(), thread_id)
        .build_async()
        .await
        .unwrap();
    let result = runtime.run(prompt.clone()).await;
    assert_eq!(
        result.expect("compacted current input commits").text,
        FINAL_TEXT
    );
    let session_dir = runtime.session_dir().unwrap().to_path_buf();
    drop(runtime);
    let cold = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    let first_turn_id = cold
        .records
        .iter()
        .find_map(|record| {
            matches!(record.entry, JournalEntry::TurnOpened(_))
                .then_some(record.turn_id)
                .flatten()
        })
        .unwrap();
    assert!(cold.unsettled_turns.is_empty());
    assert!(cold.records.iter().any(|record| matches!(&record.entry,
        JournalEntry::HistoryMutated(mutation) if mutation.messages.iter().any(|message| message.display_text() == prompt)
    )), "original accepted input remains in the journal");
    let compacted = cold
        .history
        .iter()
        .find(|message| message.display_text().starts_with("BEGIN:"))
        .unwrap();
    let compacted_text = compacted.display_text();
    // Pinned Codex keeps 20,000 approximate tokens, truncating the middle.
    assert_eq!(
        compacted_text,
        format!(
            "{}…{} tokens truncated…{}",
            &prompt[..40_000],
            (prompt.len() - 80_000).div_ceil(4),
            &prompt[prompt.len() - 40_000..]
        )
    );
    let source = cold
        .records
        .iter()
        .find_map(|record| match &record.entry {
            JournalEntry::HistoryMutated(mutation) => mutation
                .messages
                .iter()
                .find(|message| message.display_text() == prompt),
            _ => None,
        })
        .unwrap();
    assert_ne!(source.id, compacted.id);
    assert!(cold.records.iter().any(|record| matches!(&record.entry,
        JournalEntry::HistoryMutated(mutation) if mutation.compaction.as_ref().is_some_and(|report|
            report.user_message_replacements.iter().any(|replacement|
                replacement.source_message_id == source.id && replacement.replacement_message_id == compacted.id))
    )));
    config
        .module_config
        .get_mut("compactor")
        .unwrap()
        .insert("codex".to_owned(), json!({"trigger_tokens": 1_000_000}));
    let resumed = AgentRuntime::builder(config.clone(), root.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .resume_from_session_dir(session_dir.clone(), thread_id)
        .unwrap()
        .build_async()
        .await
        .unwrap();
    assert_eq!(
        resumed.run("Продолжай".to_owned()).await.unwrap().text,
        "Продолжено."
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 3);
    for request in &requests[1..] {
        let texts = request["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["content"][0]["text"].as_str())
            .collect::<Vec<_>>();
        assert!(texts.contains(&compacted_text.as_str()));
        assert!(!texts.contains(&prompt.as_str()));
    }
    drop(resumed);
    replay::check_compaction_before_first_request(&session_dir, &config, first_turn_id).await;
    let app = proteus_core::app_server::AgentAppServer::launch_resumed(
        config,
        root.path().to_path_buf(),
        Some(&config_path),
        session_dir,
    )
    .await
    .unwrap();
    let transcript = app.transcript().await.unwrap();
    assert_eq!(
        transcript.iter().filter(|item| item.text == prompt).count(),
        1
    );
    assert!(
        !transcript.iter().any(|item| item.text == compacted_text),
        "compacted model representation must not replace or duplicate the accepted user input in the UI"
    );
}
