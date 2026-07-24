use super::*;

async fn terminal_journal(
    status: TurnSettlementStatus,
    model_outcome: Option<ModelResponseOutcome>,
    settlement_error: &str,
) -> TestJournal {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let store =
        SessionStore::new(config_dir.path(), workspace.path(), session_id).expect("session store");
    let task = AgentTask::new("run terminal replay probe", workspace.path().to_path_buf());
    let user = CanonicalMessage::text(MessageRole::User, task.text.clone());
    let spec = probe_tool_spec();

    store
        .append_journal_entry(
            thread_id,
            Some(turn_id),
            JournalEntry::TurnOpened(TurnOpened {
                task,
                base_history_revision: 0,
                module_epoch: 4,
                config_snapshot: serde_json::to_value(snapshot(&spec)).expect("snapshot value"),
            }),
        )
        .await
        .expect("turn opened");
    store
        .append_history(thread_id, Some(turn_id), std::slice::from_ref(&user))
        .await
        .expect("user history");

    let exchange_id = new_exchange_id();
    store
        .append_journal_entry(
            thread_id,
            Some(turn_id),
            JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                exchange_id,
                request: recorded_request(session_id, thread_id, turn_id, vec![user], spec),
            }),
        )
        .await
        .expect("model request");
    if let Some(outcome) = model_outcome {
        store
            .append_journal_entry(
                thread_id,
                Some(turn_id),
                JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                    exchange_id,
                    outcome,
                }),
            )
            .await
            .expect("model response");
    }
    store
        .append_journal_entry(
            thread_id,
            Some(turn_id),
            JournalEntry::TurnSettled(TurnSettled {
                status,
                output: None,
                error: Some(settlement_error.to_owned()),
            }),
        )
        .await
        .expect("turn settled");

    TestJournal {
        _config_dir: config_dir,
        _workspace: workspace,
        store,
        thread_id,
        turn_id,
    }
}

#[tokio::test]
async fn terminal_workflow_error_replays_as_a_matching_outcome() {
    let model_error = "recorded provider failure";
    let settlement_error = "workflow plugin error: model stream error: recorded provider failure";
    let journal = terminal_journal(
        TurnSettlementStatus::Error,
        Some(ModelResponseOutcome::Error {
            message: model_error.to_owned(),
        }),
        settlement_error,
    )
    .await;

    let report = replay_workflow(
        journal.store.session_dir(),
        &AppConfig::default(),
        &catalog(false),
        WorkflowReplayOptions::default(),
    )
    .await
    .expect("terminal error replay");

    assert!(report.comparison.matched, "{:?}", report.comparison.issues);
    assert_eq!(report.recorded.status, TurnSettlementStatus::Error);
    assert_eq!(report.replay.status, TurnSettlementStatus::Error);
    assert_eq!(report.comparison.error_equal, Some(true));
    assert_eq!(report.comparison.output_equal, None);
    assert_eq!(report.comparison.history_equal, Some(true));
    assert_eq!(report.model_exchanges.recorded, 1);
    assert_eq!(report.model_exchanges.replayed, 1);
    assert!(report.source_journal_unchanged);
}

#[tokio::test]
async fn canceled_and_timeout_turns_fail_closed_before_incomplete_exchange_selection() {
    for (status, expected) in [
        (
            TurnSettlementStatus::Canceled,
            "external cancellation timing",
        ),
        (
            TurnSettlementStatus::Timeout,
            "runtime-owned timeout boundary",
        ),
    ] {
        let journal = terminal_journal(status, None, "runtime-owned terminal boundary").await;
        let before = std::fs::read(journal.store.journal_path()).expect("journal before");

        let error = replay_workflow(
            journal.store.session_dir(),
            &AppConfig::default(),
            &catalog(false),
            WorkflowReplayOptions::default(),
        )
        .await
        .expect_err("runtime-owned terminal status must fail closed");

        let after = std::fs::read(journal.store.journal_path()).expect("journal after");
        assert!(error.to_string().contains(expected), "{error:#}");
        assert!(error.to_string().contains("cold /history"), "{error:#}");
        assert_eq!(before, after);
    }
}
