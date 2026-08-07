use crate::{
    contracts::{CompactionInput, CompactionOutput},
    core::RuntimeCompactionHost,
    domain::HistoryCompactionReport,
};

use super::*;

const COMPACTION_WORKFLOW_ID: &str = "replay.compaction_probe";
const COMPACTION_REASON: &str = "workflow_replay_test";

#[test]
fn compaction_comparison_ignores_only_duplicated_derived_metadata() {
    let mut expected = HistoryCompactionReport::unchanged(3, Some("test".to_owned()));
    expected.changed = true;
    expected.output_messages = 2;
    expected.original_token_estimate = Some(300);
    expected.output_token_estimate = Some(30);
    let mut replay = expected.clone();
    replay.metadata = json!({
        "input_messages": 3,
        "output_messages": 2,
        "original_token_estimate": 300,
        "output_token_estimate": 30
    });
    assert!(super::super::normalize::changed_compactions_equal(
        std::slice::from_ref(&replay),
        std::slice::from_ref(&expected),
    ));

    replay.metadata = json!({ "module_signal": "changed" });
    assert!(!super::super::normalize::changed_compactions_equal(
        std::slice::from_ref(&replay),
        std::slice::from_ref(&expected),
    ));
}

#[derive(Clone, Copy)]
struct CompactionProbeWorkflow {
    mode: CompactionProbeMode,
}

#[derive(Clone, Copy)]
enum CompactionProbeMode {
    Match,
    DivergeReport,
    InvalidHistory,
}

#[async_trait]
impl Workflow for CompactionProbeWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: RuntimeContext,
    ) -> anyhow::Result<WorkflowOutput> {
        let compaction_input = replay_compaction_input(&task, &history, &ctx.model_ref);
        let compacted = ctx
            .compactor
            .compact(
                compaction_input.clone(),
                Arc::new(RuntimeCompactionHost::new(ctx.clone())),
            )
            .await?;
        let mut report =
            HistoryCompactionReport::from_compaction_output(&compaction_input, &compacted);
        if matches!(self.mode, CompactionProbeMode::DivergeReport) {
            report.summary_source = Some("changed_implementation".to_owned());
        }
        let request = CanonicalModelRequest::new(ctx.model_ref.clone(), compacted.messages.clone())
            .with_tools(vec![probe_tool_spec()]);
        let response = ctx.model.complete(request).await?;
        Ok(
            WorkflowOutput::new(AgentOutput::text("compacted"), vec![response.message])
                .with_history_replacement(compacted.messages)
                .with_compactions(
                    if matches!(self.mode, CompactionProbeMode::InvalidHistory) {
                        Vec::new()
                    } else {
                        vec![report]
                    },
                ),
        )
    }
}

fn replay_compaction_input(
    task: &AgentTask,
    history: &[CanonicalMessage],
    model_ref: &ModelRef,
) -> CompactionInput {
    CompactionInput::new(task.clone(), model_ref.clone(), history.to_vec())
        .with_reason(COMPACTION_REASON)
        .with_token_estimate(Some(400))
}

fn compaction_catalog(mode: CompactionProbeMode) -> ModuleCatalog {
    let mut catalog = ModuleCatalog::new();
    catalog.register_test_workflow(
        COMPACTION_WORKFLOW_ID,
        Arc::new(CompactionProbeWorkflow { mode }),
    );
    catalog.register_test_policy(POLICY_ID, Arc::new(ReplayAllowAll));
    catalog
}

async fn compacted_journal() -> TestJournal {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let store =
        SessionStore::new(config_dir.path(), workspace.path(), session_id).expect("session store");
    let task = AgentTask::new(
        "run compaction replay probe",
        workspace.path().to_path_buf(),
    );
    let user = CanonicalMessage::text(MessageRole::User, task.text.clone());
    let spec = probe_tool_spec();
    let mut recorded_snapshot = snapshot(&spec);
    recorded_snapshot.modules.workflow = Some(COMPACTION_WORKFLOW_ID.to_owned());

    store
        .append_journal_entry(
            thread_id,
            Some(turn_id),
            JournalEntry::TurnOpened(TurnOpened {
                task: task.clone(),
                base_history_revision: 0,
                module_epoch: 5,
                config_snapshot: serde_json::to_value(recorded_snapshot).expect("snapshot value"),
            }),
        )
        .await
        .expect("turn opened");
    store
        .append_history(thread_id, Some(turn_id), std::slice::from_ref(&user))
        .await
        .expect("user history");

    let compaction_input = replay_compaction_input(
        &task,
        std::slice::from_ref(&user),
        &ModelRef::new("missing-provider", "offline-model"),
    );
    let summary = CanonicalMessage::text(MessageRole::User, "recorded compacted summary");
    let compacted_messages = vec![summary, user];
    let mut compaction_output = CompactionOutput::changed(
        compacted_messages.clone(),
        Some("recorded compacted summary".to_owned()),
    );
    compaction_output.token_estimate = Some(40);
    compaction_output.metadata = json!({
        "input_messages": 1,
        "output_messages": 2,
        "original_token_estimate": 400,
        "output_token_estimate": 40,
        "trigger_tokens": 100,
        "summary_source": "test_fixture"
    });
    let report =
        HistoryCompactionReport::from_compaction_output(&compaction_input, &compaction_output);
    let response = CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, "compacted response"),
        Vec::new(),
        FinishReason::Stop,
    );
    append_exchange(
        &store,
        thread_id,
        turn_id,
        recorded_request(
            session_id,
            thread_id,
            turn_id,
            compacted_messages.clone(),
            spec,
        ),
        response.clone(),
    )
    .await;

    let mut final_history = compacted_messages;
    final_history.push(response.message);
    store
        .replace_history(thread_id, Some(turn_id), &final_history, Some(report))
        .await
        .expect("compacted history");
    store
        .append_journal_entry(
            thread_id,
            Some(turn_id),
            JournalEntry::TurnSettled(TurnSettled {
                status: TurnSettlementStatus::Success,
                output: Some(AgentOutput::text("compacted")),
                error: None,
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
async fn changed_compaction_replays_the_recorded_history_replacement() {
    let journal = compacted_journal().await;
    let before = std::fs::read(journal.store.journal_path()).expect("journal before");

    let report = replay_workflow(
        journal.store.session_dir(),
        &AppConfig::default(),
        &compaction_catalog(CompactionProbeMode::Match),
        WorkflowReplayOptions::default(),
    )
    .await
    .expect("changed compaction replay");

    let after = std::fs::read(journal.store.journal_path()).expect("journal after");
    assert!(report.comparison.matched, "{:?}", report.comparison.issues);
    assert_eq!(report.comparison.history_equal, Some(true));
    assert_eq!(report.comparison.output_equal, Some(true));
    assert_eq!(report.model_exchanges.recorded, 1);
    assert_eq!(report.model_exchanges.replayed, 1);
    assert_eq!(before, after);
}

#[tokio::test]
async fn changed_compaction_report_divergence_is_not_hidden_by_equal_history() {
    let journal = compacted_journal().await;

    let report = replay_workflow(
        journal.store.session_dir(),
        &AppConfig::default(),
        &compaction_catalog(CompactionProbeMode::DivergeReport),
        WorkflowReplayOptions::default(),
    )
    .await
    .expect("compaction report divergence");

    assert!(!report.comparison.matched);
    assert_eq!(report.comparison.history_equal, Some(true));
    assert_eq!(report.comparison.output_equal, Some(true));
    assert!(report.comparison.issues.iter().any(|issue| {
        issue.contains("changed compaction reports differ from the canonical journal")
    }));
    assert!(report.source_journal_unchanged);
}

#[tokio::test]
async fn replay_uses_runtime_history_validation_for_workflow_output() {
    let journal = compacted_journal().await;

    let report = replay_workflow(
        journal.store.session_dir(),
        &AppConfig::default(),
        &compaction_catalog(CompactionProbeMode::InvalidHistory),
        WorkflowReplayOptions::default(),
    )
    .await
    .expect("invalid history divergence report");

    assert!(!report.comparison.matched);
    assert_eq!(report.replay.status, TurnSettlementStatus::Error);
    assert!(
        report.replay.error.as_deref().is_some_and(|error| {
            error.contains("history replacement without changed compaction")
        })
    );
    assert!(report.source_journal_unchanged);
}
