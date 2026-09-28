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

async fn check_large_current_input_preflight(cold_preflight: bool) {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = config(&format!("http://{}", listener.local_addr().unwrap()));
    let prompt = format!("BEGIN:{}:END", "abcdefghij".repeat(10_000));
    let incoming = format!("NEW:{}:END", "klmnopqrst".repeat(10_000));
    config
        .module_config
        .get_mut("model")
        .unwrap()
        .get_mut("openai")
        .unwrap()["max_input_tokens"] = json!(100_000);
    config
        .module_config
        .get_mut("compactor")
        .unwrap()
        .get_mut("codex")
        .unwrap()["trigger_tokens"] = json!(90_000);
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let server = tokio::spawn(serve(
        listener,
        vec![
            FixtureReply::Json(completed_response(
                "prior",
                vec![assistant("final_answer", "Prior completed")],
                Some(json!({"input_tokens": 94_900, "output_tokens": 100})),
            )),
            FixtureReply::Json(completed_response(
                "summary",
                vec![assistant("final_answer", SUMMARY_TEXT)],
                None,
            )),
            FixtureReply::Json(completed_response(
                "final",
                vec![assistant("final_answer", FINAL_TEXT)],
                Some(json!({"input_tokens": 500, "output_tokens": 20})),
            )),
            FixtureReply::Json(completed_response(
                "continued",
                vec![assistant("final_answer", "Продолжено.")],
                None,
            )),
        ],
    ));
    let thread_id = proteus_contracts::domain::new_thread_id();
    let mut runtime = AgentRuntime::builder(config.clone(), root.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_session_ids(new_session_id(), thread_id)
        .build_async()
        .await
        .unwrap();
    assert_eq!(
        runtime.run(prompt.clone()).await.unwrap().text,
        "Prior completed"
    );
    let session_dir = runtime.session_dir().unwrap().to_path_buf();
    if cold_preflight {
        drop(runtime);
        runtime = AgentRuntime::builder(config.clone(), root.path().to_path_buf())
            .with_config_path(Some(&config_path))
            .resume_from_session_dir(session_dir.clone(), thread_id)
            .unwrap()
            .build_async()
            .await
            .unwrap();
    }
    assert_eq!(
        runtime.run(incoming.clone()).await.unwrap().text,
        FINAL_TEXT
    );
    drop(runtime);
    let cold = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    let compacted_turn_id = cold
        .records
        .iter()
        .filter_map(|record| {
            matches!(record.entry, JournalEntry::TurnOpened(_))
                .then_some(record.turn_id)
                .flatten()
        })
        .nth(1)
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
    let current = cold
        .history
        .iter()
        .find(|message| message.display_text() == incoming)
        .unwrap();
    let accepted_current = cold
        .records
        .iter()
        .find_map(|record| match &record.entry {
            JournalEntry::HistoryMutated(mutation) => mutation
                .messages
                .iter()
                .find(|message| message.display_text() == incoming),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        current, accepted_current,
        "the incoming user retains exact content and identity"
    );
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
    assert_eq!(requests.len(), 4);
    let texts = |request: &Value| {
        request["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["content"][0]["text"].as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    assert!(texts(&requests[0]).contains(&prompt));
    // Both histories are below the trigger by byte estimation. Previous real
    // usage must trigger the preflight on warm and reconstructed cold state.
    assert!(serde_json::to_string(&requests[0]).unwrap().len() / 4 < 90_000);
    let summary_texts = texts(&requests[1]);
    assert!(summary_texts.contains(&prompt));
    assert!(!summary_texts.contains(&incoming));
    assert_eq!(summary_texts.last().unwrap(), COMPACTION_PROMPT);
    for request in &requests[2..] {
        let texts = texts(request);
        assert!(texts.contains(&compacted_text));
        assert!(texts.contains(&incoming));
        assert!(!texts.contains(&prompt));
    }
    assert_eq!(texts(&requests[2]).last().unwrap(), &incoming);
    drop(resumed);
    replay::check_compaction_before_first_request(&session_dir, &config, compacted_turn_id).await;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prior_usage_preflight_preserves_large_incoming_user_on_warm_turn() {
    check_large_current_input_preflight(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prior_usage_preflight_preserves_large_incoming_user_on_cold_resume() {
    check_large_current_input_preflight(true).await;
}
