use std::path::Path;

use proteus_contracts::{contracts::ModelCallOrigin, domain::TurnId};
use proteus_core::core::{
    AppConfig, JournalEntry, ModuleCatalog, SessionStore, TurnSettlementStatus,
    WorkflowReplayOptions, read_eval_report, replay_workflow,
};

pub(super) async fn check(
    session_dir: &Path,
    workspace: &Path,
    config: &AppConfig,
    context_window_once: bool,
) {
    let projection = SessionStore::open(session_dir.to_owned())
        .expect("cold session")
        .load_projection()
        .expect("cold projection");
    let origins: Vec<_> = projection
        .records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::ModelRequestRecorded(request) => Some(request.origin),
            _ => None,
        })
        .collect();
    let mut expected_origins = vec![ModelCallOrigin::Direct, ModelCallOrigin::Compactor];
    if context_window_once {
        expected_origins.push(ModelCallOrigin::Compactor);
    }
    expected_origins.push(ModelCallOrigin::Direct);
    assert_eq!(origins, expected_origins);
    let eval = read_eval_report(session_dir).expect("recorded model usage");
    assert_eq!(eval.model_calls, expected_origins.len());
    assert_eq!(eval.provider_input_tokens, 12_800);
    assert_eq!(eval.provider_output_tokens, 460);

    // The HTTP fixture has closed and the real tool input is gone. Replay
    // must use recorded model outcomes, the compaction result and tool output.
    std::fs::remove_file(workspace.join("probe.txt")).expect("remove live tool input");
    let report = replay_workflow(
        session_dir,
        config,
        &ModuleCatalog::from_config(config).expect("replay catalog"),
        WorkflowReplayOptions::default(),
    )
    .await
    .expect("compaction workflow replay");
    assert!(report.comparison.matched, "{report:#?}");
    assert_eq!(report.replay.status, TurnSettlementStatus::Success);
    assert_eq!(report.comparison.history_equal, Some(true));
    assert_eq!(report.comparison.output_equal, Some(true));
    assert_eq!(report.model_exchanges.recorded, 2);
    assert_eq!(report.model_exchanges.replayed, 2);
    assert_eq!(report.tool_calls.recorded, 1);
    assert_eq!(report.tool_calls.replayed, 1);
    assert!(report.source_journal_unchanged);
}

pub(super) async fn check_compaction_before_first_request(
    session_dir: &Path,
    config: &AppConfig,
    turn_id: TurnId,
) {
    let report = replay_workflow(
        session_dir,
        config,
        &ModuleCatalog::from_config(config).expect("replay catalog"),
        WorkflowReplayOptions {
            turn_id: Some(turn_id),
        },
    )
    .await
    .expect("initial compaction replay");
    assert!(report.comparison.matched, "{report:#?}");
    assert_eq!(report.replay.status, TurnSettlementStatus::Success);
    assert_eq!(report.model_exchanges.recorded, 1);
    assert_eq!(report.model_exchanges.replayed, 1);
    assert_eq!(report.tool_calls.recorded, 0);
    assert!(report.source_journal_unchanged);
}
