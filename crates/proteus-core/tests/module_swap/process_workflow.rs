use super::*;

fn python_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn reference_path() -> std::path::PathBuf {
    workspace_root_file("examples/modules/agent-worker/agent.py")
}

fn process_workflow_config() -> AppConfig {
    let mut config = test_config();
    config.modules.workflow = "process".to_owned();
    config.modules.policy = "allow_all".to_owned();
    config.modules.subagent = "none".to_owned();
    set_module_config(
        &mut config,
        "workflow",
        "process",
        json!({
            "module_id": "python_agent_loop",
            "command": "python3",
            "args": [reference_path().display().to_string()],
            "handshake_timeout_ms": 3000,
            "config": {
                "max_tool_rounds": 4,
                "system_instructions": "Run the protocol integration fixture.",
            },
        }),
    );
    config
}

#[tokio::test]
async fn workflow_slot_swaps_none_and_external_process_without_runtime_changes() {
    if !python_available() {
        return;
    }
    let mut none_config = test_config();
    none_config.modules.workflow = "none".to_owned();
    let (none_text, _) = run_with(none_config, "hello").await;
    assert!(none_text.contains("workflow is disabled"), "{none_text}");

    let (process_text, _) = run_with(process_workflow_config(), "hello").await;
    assert!(process_text.contains("Fake final answer"), "{process_text}");
}

#[tokio::test]
async fn external_workflow_runs_real_model_tool_loop_through_host_policy_path() {
    if !python_available() {
        return;
    }

    let (text, events) = run_with(
        process_workflow_config(),
        "remember_fact process workflow evidence",
    )
    .await;

    assert!(
        text.contains("Fake final answer after tool result"),
        "{text}"
    );
    let records = events.events().await;
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::ToolCallRequested { call } if call.name == "remember_fact"
        )
    }));
    assert!(
        records
            .iter()
            .any(|event| { matches!(event, Event::ToolFinished { result } if result.ok) })
    );
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::TurnFinished { output }
                if output.metadata["workflow"] == "python_agent_loop"
        )
    }));
    assert!(
        records.iter().any(|event| {
            matches!(event, Event::AssistantTextDelta { text } if !text.is_empty())
        })
    );
}

#[tokio::test]
async fn external_workflow_cannot_bypass_selected_policy() {
    if !python_available() {
        return;
    }
    let mut config = process_workflow_config();
    config.modules.policy = "deny_all".to_owned();

    let (text, events) = run_with(config, "remember_fact denied process workflow").await;

    assert!(
        text.contains("Fake final answer after tool result"),
        "{text}"
    );
    let records = events.events().await;
    assert!(records.iter().any(|event| {
        matches!(
            event,
            Event::ToolFinished { result }
                if !result.ok
                    && result.error.as_deref().is_some_and(|error| error.contains("policy"))
        )
    }));
}

#[tokio::test]
async fn process_workflow_turn_is_durable_and_resumes_with_a_fresh_worker() {
    if !python_available() {
        return;
    }
    let dir = temp_workspace();
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, "").expect("config file");
    let config = process_workflow_config();
    let runtime = AgentRuntime::builder(config.clone(), dir.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .expect("process workflow runtime");

    let first = runtime
        .run("remember_fact durable process workflow".to_owned())
        .await
        .expect("first process workflow turn");
    let session_dir = runtime.session_dir().expect("session dir").to_path_buf();
    let store = SessionStore::open(session_dir.clone()).expect("session store");
    let records = store.load_records().expect("canonical journal");
    let thread_id = records.first().expect("journal record").thread_id;

    assert!(first.text.contains("Fake final answer"), "{}", first.text);

    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(
                record.entry,
                proteus_core::core::JournalEntry::ModelRequestRecorded(_)
            ))
            .count(),
        2
    );
    assert!(records.iter().any(|record| {
        matches!(
            &record.entry,
            proteus_core::core::JournalEntry::ToolCallRecorded(call)
                if matches!(call.phase, proteus_core::core::ToolCallRecordPhase::Requested)
                    && call.call.name == "remember_fact"
        )
    }));
    assert!(records.iter().any(|record| {
        matches!(
            &record.entry,
            proteus_core::core::JournalEntry::ToolResultRecorded(result) if result.result.ok
        )
    }));
    assert!(records.iter().any(|record| {
        matches!(
            &record.entry,
            proteus_core::core::JournalEntry::TurnSettled(settled)
                if settled.status == proteus_core::core::TurnSettlementStatus::Success
                    && settled.output.as_ref().is_some_and(|output| {
                        output.metadata["workflow"] == "python_agent_loop"
                    })
        )
    }));
    assert_eq!(store.load_messages().expect("cold history").len(), 4);

    let replay = proteus_core::core::replay_workflow(
        store.session_dir(),
        &config,
        &test_catalog(),
        proteus_core::core::WorkflowReplayOptions::default(),
    )
    .await
    .expect("process workflow replay");
    assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
    assert!(replay.source_journal_unchanged);
    assert_eq!(replay.model_exchanges.recorded, 2);
    assert_eq!(replay.model_exchanges.replayed, 2);
    assert_eq!(replay.tool_calls.recorded, 1);
    assert_eq!(replay.tool_calls.replayed, 1);

    drop(runtime);
    let resumed = AgentRuntime::builder(config, dir.path().to_path_buf())
        .resume_from_session_dir(session_dir.clone(), thread_id)
        .expect("resume builder")
        .with_module_catalog(test_catalog())
        .build()
        .expect("resumed process workflow runtime");
    let second = resumed
        .run("continue after worker restart".to_owned())
        .await
        .expect("resumed process workflow turn");

    assert!(second.text.contains("Fake final answer"), "{}", second.text);
    assert_eq!(resumed.history_len().await, 6);
    assert_eq!(
        SessionStore::open(session_dir)
            .expect("reopened store")
            .load_messages()
            .expect("cold resumed history")
            .len(),
        6
    );
}

#[tokio::test]
async fn process_workflow_timeout_has_canonical_settlement_and_cold_history() {
    if !python_available() {
        return;
    }
    let dir = temp_workspace();
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, "").expect("config file");
    let mut config = process_workflow_config();
    config.runtime.workflow_timeout_ms = 40;
    config
        .providers
        .get_mut(&config.active_provider)
        .expect("active fake provider")
        .provider_config = json!({ "stream_delay_ms": 200 });
    let runtime = AgentRuntime::builder(config, dir.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .expect("process workflow runtime");

    let error = runtime
        .run("slow process workflow".to_owned())
        .await
        .expect_err("outer workflow deadline must win");
    assert!(
        error.to_string().contains("workflow timed out after 40ms"),
        "{error:#}"
    );

    tokio::time::sleep(Duration::from_millis(100)).await;
    let store = SessionStore::open(runtime.session_dir().expect("session dir").to_path_buf())
        .expect("session store");
    let records = store.load_records().expect("canonical journal");
    let turn_records = records
        .iter()
        .filter(|record| record.turn_id.is_some())
        .collect::<Vec<_>>();
    assert!(matches!(
        turn_records.last().map(|record| &record.entry),
        Some(proteus_core::core::JournalEntry::TurnSettled(settled))
            if settled.status == proteus_core::core::TurnSettlementStatus::Timeout
    ));
    let cold_history = store.load_messages().expect("cold timeout history");
    assert_eq!(cold_history.len(), 1);
    assert_eq!(cold_history[0].role, MessageRole::User);
}

#[tokio::test]
async fn process_workflow_cancel_has_canonical_settlement_and_cold_history() {
    if !python_available() {
        return;
    }
    let dir = temp_workspace();
    let config_dir = tempfile::tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, "").expect("config file");
    let mut config = process_workflow_config();
    config.runtime.workflow_timeout_ms = 0;
    config
        .providers
        .get_mut(&config.active_provider)
        .expect("active fake provider")
        .provider_config = json!({ "stream_delay_ms": 200 });
    let runtime = AgentRuntime::builder(config, dir.path().to_path_buf())
        .with_config_path(Some(&config_path))
        .with_module_catalog(test_catalog())
        .build()
        .expect("process workflow runtime");
    let cancellation = proteus_core::contracts::CancellationToken::new();
    let signal = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        signal.cancel();
    });

    let error = runtime
        .run_with_cancellation("cancel process workflow".to_owned(), cancellation)
        .await
        .expect_err("canceled worker must not finish the turn");
    assert!(error.to_string().contains("canceled"), "{error:#}");

    let store = SessionStore::open(runtime.session_dir().expect("session dir").to_path_buf())
        .expect("session store");
    let records = store.load_records().expect("canonical journal");
    let settlement = records.iter().rev().find_map(|record| match &record.entry {
        proteus_core::core::JournalEntry::TurnSettled(settled) => Some(settled),
        _ => None,
    });
    assert!(matches!(
        settlement,
        Some(settled) if settled.status == proteus_core::core::TurnSettlementStatus::Canceled
    ));
    let cold_history = store.load_messages().expect("cold canceled history");
    assert_eq!(cold_history.len(), 1);
    assert_eq!(cold_history[0].role, MessageRole::User);
}

#[test]
fn workflow_handshake_mismatch_is_a_snapshot_build_error() {
    if !python_available() {
        return;
    }
    let dir = temp_workspace();
    let mut config = process_workflow_config();
    set_module_config(
        &mut config,
        "workflow",
        "process",
        json!({
            "module_id": "wrong_agent_id",
            "command": "python3",
            "args": [reference_path().display().to_string()],
            "handshake_timeout_ms": 3000,
        }),
    );

    let error =
        match RuntimeRegistry::from_catalog(&config, dir.path().to_path_buf(), test_catalog()) {
            Ok(_) => panic!("mismatched workflow identity must not build"),
            Err(error) => error,
        };

    let message = format!("{error:#}");
    assert!(message.contains("handshake failed"), "{message}");
    assert!(
        message.contains("unsupported initialize identity"),
        "{message}"
    );
}
