use super::*;

#[tokio::test]
async fn runtime_keeps_session_id_and_creates_new_turn_id_per_run() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_event_sink(events.clone())
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let first = runtime.run("summarize first".to_owned()).await.unwrap();
    let second = runtime.run("summarize second".to_owned()).await.unwrap();
    let records = events.events().await;

    let session_ids = records
        .iter()
        .filter_map(|event| match event {
            Event::SessionStarted { session_id, .. } => Some(*session_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let turn_ids = records
        .iter()
        .filter_map(|event| match event {
            Event::TurnStarted {
                session_id,
                turn_id,
                ..
            } => Some((*session_id, *turn_id)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(session_ids.len(), 1);
    assert_eq!(turn_ids.len(), 2);
    assert_eq!(turn_ids[0].0, session_ids[0]);
    assert_eq!(turn_ids[1].0, session_ids[0]);
    assert_ne!(turn_ids[0].1, turn_ids[1].1);
    assert_eq!(first.metadata["session_id"], session_ids[0].to_string());
    assert_eq!(second.metadata["session_id"], session_ids[0].to_string());
    assert_ne!(first.metadata["turn_id"], second.metadata["turn_id"]);
}

#[tokio::test]
async fn runtime_builder_can_reuse_existing_session_and_thread_ids() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_event_sink(events.clone())
        .with_session_ids(session_id, thread_id)
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let output = runtime
        .run("summarize reused ids".to_owned())
        .await
        .unwrap();
    let records = events.events().await;

    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::SessionStarted { session_id: id, .. } if *id == session_id
        )
    }));
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::TurnStarted {
                session_id: sid,
                thread_id: tid,
                ..
            } if *sid == session_id && *tid == thread_id
        )
    }));
    assert_eq!(output.metadata["session_id"], session_id.to_string());
    assert_eq!(output.metadata["thread_id"], thread_id.to_string());
}

#[tokio::test]
async fn fanout_preserves_event_envelope_identity_across_sinks() {
    let left = Arc::new(InMemoryEventStore::new());
    let right = Arc::new(InMemoryEventStore::new());
    let emitter = EventEmitter::new(Arc::new(FanoutEventSink::new(vec![
        left.clone(),
        right.clone(),
    ])));
    let session_id = new_session_id();
    let thread_id = new_thread_id();

    emitter
        .emit(
            EventContext::new(session_id, thread_id, None),
            Event::Error {
                message: "same envelope".to_owned(),
            },
        )
        .await
        .unwrap();

    let left = left.envelopes().await;
    let right = right.envelopes().await;

    assert_eq!(left.len(), 1);
    assert_eq!(right.len(), 1);
    assert_eq!(left[0].event_id, right[0].event_id);
    assert_eq!(left[0].seq, 1);
    assert_eq!(right[0].seq, 1);
    assert_eq!(left[0].session_id, session_id);
    assert_eq!(left[0].thread_id, thread_id);
    assert_eq!(left[0].turn_id, None);
    assert_eq!(left[0].schema_version, 1);
}
// folder_listing_question_uses_list_dir_context was removed together with the
// old directory-listing heuristic. The feature assumed list_dir was a builtin,
// which it no longer is.

#[tokio::test]
async fn context_chunks_are_not_persisted_to_runtime_history() {
    let dir = temp_workspace();
    let events = Arc::new(InMemoryEventStore::new());
    let runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_event_sink(events)
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let first = runtime.run("hello".to_owned()).await.unwrap();
    let follow_up = runtime.run("summarize".to_owned()).await.unwrap();

    assert!(first.text.contains("Fake final answer"));
    assert!(follow_up.text.contains("Fake final answer"));
    // History contains only conversational messages (user+assistant per turn),
    // not the ephemeral context chunks.
    assert_eq!(runtime.history_len().await, 4);
}

#[tokio::test]
async fn context_chunks_are_not_written_to_session_store() {
    let dir = temp_workspace();
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, "").expect("config file");
    let runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let output = runtime.run("hello".to_owned()).await.unwrap();
    let messages_path = runtime.session_dir().unwrap().join("messages.jsonl");
    let contents = std::fs::read_to_string(messages_path).expect("messages jsonl");
    let messages = contents
        .lines()
        .map(|line| serde_json::from_str::<CanonicalMessage>(line).expect("message"))
        .collect::<Vec<_>>();

    assert!(output.text.contains("Fake final answer"));
    assert_eq!(messages.len(), 2);
    // Ephemeral context chunks (from the simple context builder) must not
    // leak into the persistent session transcript.
    assert!(!messages.iter().any(|message| {
        message
            .parts
            .iter()
            .any(|part| matches!(part, ContentPart::Context { .. }))
    }));
}

#[tokio::test]
async fn runtime_writes_relative_event_log_under_config_root() {
    let dir = temp_workspace();
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("configs").join("config.toml");
    std::fs::create_dir_all(config_path.parent().unwrap()).expect("config parent");
    std::fs::write(&config_path, "").expect("config file");
    let runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    runtime.run("hello".to_owned()).await.unwrap();

    assert!(config_dir.path().join(".proteus/events.jsonl").exists());
    assert!(!dir.path().join(".proteus/events.jsonl").exists());
}

#[tokio::test]
async fn runtime_can_resume_history_from_existing_session_dir() {
    let dir = temp_workspace();
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, "").expect("config file");
    let first_runtime = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let first = first_runtime
        .run("summarize before resume".to_owned())
        .await
        .unwrap();
    let session_dir = first_runtime.session_dir().unwrap().to_path_buf();
    let session_id = first.metadata["session_id"]
        .as_str()
        .expect("session id")
        .parse()
        .expect("session uuid");
    let thread_id = first.metadata["thread_id"]
        .as_str()
        .expect("thread id")
        .parse()
        .expect("thread uuid");

    let resumed = AgentRuntime::builder(test_config(), dir.path().to_path_buf())
        .resume_from_session_dir(session_dir.clone(), session_id, thread_id)
        .unwrap()
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();
    assert_eq!(resumed.history_len().await, 2);

    let second = resumed
        .run("summarize after resume".to_owned())
        .await
        .unwrap();
    assert_eq!(second.metadata["session_id"], session_id.to_string());
    assert_eq!(second.metadata["thread_id"], thread_id.to_string());
    assert_eq!(resumed.history_len().await, 4);

    let messages_path = session_dir.join("messages.jsonl");
    let lines = std::fs::read_to_string(messages_path)
        .expect("messages jsonl")
        .lines()
        .count();
    assert_eq!(lines, 4);
}

#[tokio::test]
async fn runtime_resume_uses_workspace_from_session_metadata() {
    let original_dir = temp_workspace();
    let wrong_dir = temp_workspace();
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, "").expect("config file");
    let first_runtime = AgentRuntime::builder(test_config(), original_dir.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    let first = first_runtime
        .run("create persisted session".to_owned())
        .await
        .unwrap();
    let session_dir = first_runtime.session_dir().unwrap().to_path_buf();
    let session_id = first.metadata["session_id"]
        .as_str()
        .expect("session id")
        .parse()
        .expect("session uuid");
    let thread_id = first.metadata["thread_id"]
        .as_str()
        .expect("thread id")
        .parse()
        .expect("thread uuid");
    let resumed = AgentRuntime::builder(test_config(), wrong_dir.path().to_path_buf())
        .resume_from_session_dir(session_dir, session_id, thread_id)
        .unwrap()
        .with_module_catalog(test_catalog())
        .build()
        .unwrap();

    assert_eq!(resumed.cwd(), original_dir.path());
}
