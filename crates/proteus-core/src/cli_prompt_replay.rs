use anyhow::Result;

use proteus_core::core::{
    AppConfig, ModuleCatalog, PromptReplayCounts, PromptReplayNames, PromptReplayOptions,
    PromptReplayOutcomeSummary, PromptReplayReport, PromptReplayUsage, replay_prompt,
};

use crate::cli_commands::PromptReplayCommand;

pub(crate) async fn run_prompt_replay(
    config: &AppConfig,
    command: PromptReplayCommand,
) -> Result<String> {
    let model_config = config.active_model_config()?;
    let catalog = ModuleCatalog::from_config(config)?;
    let adapter = catalog.build_model_adapter(&model_config)?;
    let report = replay_prompt(
        command.source,
        adapter,
        PromptReplayOptions {
            exchange_id: command.exchange_id,
            allow_hosted_tools: command.allow_hosted_tools,
        },
    )
    .await?;
    render_prompt_replay_report(&report, command.json)
}

pub(crate) fn render_prompt_replay_report(
    report: &PromptReplayReport,
    json: bool,
) -> Result<String> {
    if json {
        return serde_json::to_string_pretty(report).map_err(Into::into);
    }
    Ok(render_human_report(report))
}

fn render_human_report(report: &PromptReplayReport) -> String {
    let mut lines = vec![
        "Prompt replay report".to_owned(),
        format!("Source: {}", report.source.journal_path.display()),
        format!("Session: {}", report.source.session_id),
        format!("Exchange: {}", report.source.exchange_id),
        format!("Thread: {}", report.source.thread_id),
        format!("Turn: {}", report.source.turn_id),
        format!("Recorded model: {}", model_label(&report.recorded_model)),
        format!(
            "Replay model: {} (adapter={})",
            model_label(&report.replay_model),
            report.replay_adapter
        ),
        format!(
            "Recorded outcome: {}",
            outcome_label(&report.recorded_outcome)
        ),
        format!("Replay outcome: {}", outcome_label(&report.replay_outcome)),
        format!("Usage: {}", usage_label(&report.usage)),
        format!(
            "Text equal: {}",
            match report.text_equal {
                Some(true) => "yes",
                Some(false) => "no",
                None => "unavailable",
            }
        ),
        format!(
            "Local tool calls: {}",
            count_label(&report.local_tool_calls)
        ),
        format!(
            "Hosted activities: {}",
            count_label(&report.hosted_activities)
        ),
        format!("Citations: {}", count_label(&report.citations)),
        format!("Duration: {} ms", report.duration_ms),
    ];

    if !report.request_hosted_tools.is_empty() {
        lines.push(format!(
            "Provider-hosted tools in request: {} (opt-in={})",
            report.request_hosted_tools.join(", "),
            if report.hosted_tools_allowed {
                "enabled"
            } else {
                "disabled"
            }
        ));
    }
    append_local_tool_names(&mut lines, &report.local_tool_call_names);
    if let Some(text) = report
        .replay_outcome
        .text
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        lines.push("Replay text:".to_owned());
        lines.push(text.to_owned());
    }
    lines.join("\n")
}

fn model_label(model: &proteus_core::domain::ModelRef) -> String {
    format!("{}/{}", model.provider, model.model)
}

fn outcome_label(outcome: &PromptReplayOutcomeSummary) -> String {
    let mut label = outcome.status.as_str().to_owned();
    if let Some(finish_reason) = &outcome.finish_reason {
        label.push_str(&format!(" (finish_reason={finish_reason})"));
    }
    if let Some(error) = &outcome.error {
        label.push_str(&format!(": {error}"));
    }
    label
}

fn usage_label(usage: &PromptReplayUsage) -> String {
    format!(
        "recorded={}, replay={}",
        token_usage_label(usage.recorded.as_ref()),
        token_usage_label(usage.replay.as_ref())
    )
}

fn token_usage_label(usage: Option<&proteus_core::model_standard::TokenUsage>) -> String {
    usage.map_or_else(
        || "unavailable".to_owned(),
        |usage| {
            format!(
                "input:{} output:{}",
                usage.input_tokens, usage.output_tokens
            )
        },
    )
}

fn count_label(counts: &PromptReplayCounts) -> String {
    format!("recorded={}, replay={}", counts.recorded, counts.replay)
}

fn append_local_tool_names(lines: &mut Vec<String>, names: &PromptReplayNames) {
    if !names.recorded.is_empty() {
        lines.push(format!(
            "Recorded local tool calls: {}",
            names.recorded.join(", ")
        ));
    }
    if !names.replay.is_empty() {
        lines.push(format!(
            "Replay local tool calls (not executed): {}",
            names.replay.join(", ")
        ));
    }
}
