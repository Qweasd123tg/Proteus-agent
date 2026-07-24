use anyhow::Result;

use proteus_core::core::{
    AppConfig, WorkflowReplayOutcome, WorkflowReplayReport, load_default_module_catalog,
    replay_workflow,
};

use crate::cli_commands::WorkflowReplayCommand;

pub(crate) async fn run_workflow_replay(
    config: &AppConfig,
    command: WorkflowReplayCommand,
) -> Result<String> {
    let (catalog, _) = load_default_module_catalog();
    let report = replay_workflow(
        command.source,
        config,
        &catalog,
        proteus_core::core::WorkflowReplayOptions {
            turn_id: command.turn_id,
        },
    )
    .await?;
    render_workflow_replay_report(&report, command.json)
}

pub(crate) fn render_workflow_replay_report(
    report: &WorkflowReplayReport,
    json: bool,
) -> Result<String> {
    if json {
        return serde_json::to_string_pretty(report).map_err(Into::into);
    }
    Ok(render_human_report(report))
}

fn render_human_report(report: &WorkflowReplayReport) -> String {
    let mut lines = vec![
        "Workflow replay report".to_owned(),
        format!("Source: {}", report.source.journal_path.display()),
        format!("Session: {}", report.source.session_id),
        format!("Turn: {}", report.source.turn_id),
        format!("Thread: {}", report.source.thread_id),
        format!(
            "Workflow: {} (policy={}, epoch={})",
            report.source.workflow_id, report.source.policy_id, report.source.module_epoch
        ),
        format!("Recorded outcome: {}", outcome_label(&report.recorded)),
        format!("Replay outcome: {}", outcome_label(&report.replay)),
        format!(
            "Model exchanges: recorded={}, replayed={}",
            report.model_exchanges.recorded, report.model_exchanges.replayed
        ),
        format!(
            "Tool calls: recorded={}, replayed={}",
            report.tool_calls.recorded, report.tool_calls.replayed
        ),
        format!(
            "History equal: {}",
            optional_bool(report.comparison.history_equal)
        ),
        format!(
            "Output equal: {}",
            optional_bool(report.comparison.output_equal)
        ),
        format!(
            "Source journal unchanged: {}",
            yes_no(report.source_journal_unchanged)
        ),
        format!(
            "Status: {}",
            if report.comparison.matched {
                "matched"
            } else {
                "diverged"
            }
        ),
        format!("Duration: {} ms", report.duration_ms),
    ];
    if !report.comparison.issues.is_empty() {
        lines.push("Divergences:".to_owned());
        lines.extend(
            report
                .comparison
                .issues
                .iter()
                .map(|issue| format!("- {issue}")),
        );
    }
    if let Some(text) = report
        .replay
        .output
        .as_ref()
        .map(|output| output.text.as_str())
        .filter(|text| !text.is_empty())
    {
        lines.push("Replay text:".to_owned());
        lines.push(text.to_owned());
    }
    lines.join("\n")
}

fn outcome_label(outcome: &WorkflowReplayOutcome) -> String {
    let status = match outcome.status {
        proteus_core::core::TurnSettlementStatus::Success => "success",
        proteus_core::core::TurnSettlementStatus::Error => "error",
        proteus_core::core::TurnSettlementStatus::Canceled => "canceled",
        proteus_core::core::TurnSettlementStatus::Timeout => "timeout",
    };
    match &outcome.error {
        Some(error) => format!("{status}: {error}"),
        None => status.to_owned(),
    }
}

fn optional_bool(value: Option<bool>) -> &'static str {
    value.map_or("unavailable", yes_no)
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
