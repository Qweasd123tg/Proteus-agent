use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::{
    domain::{Event, EventEnvelope, HistoryCompactionReport, SessionId, TokenUsageSource},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

use super::{
    AppContextBuildSnapshot, AppContextCompactionSnapshot, AppContextHistorySummary,
    AppContextMapSnapshot, AppContextToolSummary, AppContextUsageCategory, AppContextUsageSnapshot,
    AppSessionActivity, paths_equal,
};

pub(super) struct ContextMapInput {
    pub session_dir: Option<PathBuf>,
    pub session_id: Option<SessionId>,
    pub workspace_path: Option<PathBuf>,
    pub activity: Option<AppSessionActivity>,
    pub history: Vec<CanonicalMessage>,
    pub event_log_path: PathBuf,
    pub diagnostics: Vec<String>,
}

pub(super) fn build_context_map_snapshot(input: ContextMapInput) -> Result<AppContextMapSnapshot> {
    let mut diagnostics = input.diagnostics;
    let events = read_context_events(
        &input.event_log_path,
        input.session_id,
        input.session_dir.as_deref(),
        &mut diagnostics,
    )?;
    let history = summarize_context_history(&input.history);
    let tools = summarize_context_tools(&events);
    let mut snapshot = AppContextMapSnapshot::new(
        input.session_dir,
        input.session_id,
        input.workspace_path,
        history,
        tools,
    );
    snapshot.activity = input.activity;
    apply_context_events(&mut snapshot, &events);
    if snapshot.latest_usage.is_none() {
        diagnostics
            .push("token usage telemetry unavailable; showing history-only fallback".to_owned());
    }
    snapshot.diagnostics = diagnostics;
    Ok(snapshot)
}

fn read_context_events(
    event_log_path: &Path,
    session_id: Option<SessionId>,
    session_dir: Option<&Path>,
    diagnostics: &mut Vec<String>,
) -> Result<Vec<EventEnvelope>> {
    let content = match std::fs::read_to_string(event_log_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            diagnostics.push("event log not found; using session history only".to_owned());
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", event_log_path.display()));
        }
    };

    let mut skipped = 0usize;
    let mut events = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<EventEnvelope>(line) {
            Ok(envelope) if context_event_matches(&envelope, session_id, session_dir) => {
                events.push(envelope);
            }
            Ok(_) => {}
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        diagnostics.push(format!("skipped {skipped} malformed event log lines"));
    }
    Ok(events)
}

fn context_event_matches(
    envelope: &EventEnvelope,
    session_id: Option<SessionId>,
    session_dir: Option<&Path>,
) -> bool {
    if let Some(session_id) = session_id {
        return envelope.session_id == session_id;
    }
    let Some(session_dir) = session_dir else {
        return false;
    };
    match &envelope.event {
        Event::SessionStarted {
            session_dir: Some(started_dir),
            ..
        } => paths_equal(started_dir, session_dir),
        _ => false,
    }
}

fn apply_context_events(snapshot: &mut AppContextMapSnapshot, events: &[EventEnvelope]) {
    for envelope in events {
        match &envelope.event {
            Event::ContextBuilt {
                chunks,
                token_estimate,
            } => {
                let mut context = AppContextBuildSnapshot::new(*chunks);
                context.token_estimate = *token_estimate;
                context.turn_id = envelope.turn_id;
                context.timestamp_ms = Some(envelope.timestamp_ms);
                snapshot.latest_context = Some(context);
            }
            Event::TokenUsageUpdated { usage } => {
                let mut usage_snapshot = AppContextUsageSnapshot::new(
                    usage.model.provider.clone(),
                    usage.model.model.clone(),
                    usage.estimated_input_tokens,
                    token_usage_source_label(usage.usage_source()),
                );
                usage_snapshot.phase = usage.phase.clone();
                usage_snapshot.max_input_tokens = usage.max_input_tokens;
                usage_snapshot.compaction_trigger_tokens = usage.compaction_trigger_tokens;
                usage_snapshot.categories = usage
                    .categories
                    .iter()
                    .map(|category| {
                        let mut category_snapshot =
                            AppContextUsageCategory::new(category.name.clone(), category.tokens);
                        if let Some(source) = category.source {
                            category_snapshot =
                                category_snapshot.with_source(token_usage_source_label(source));
                        }
                        category_snapshot
                    })
                    .collect();
                usage_snapshot.actual = usage.actual.clone();
                usage_snapshot.turn_id = envelope.turn_id;
                usage_snapshot.timestamp_ms = Some(envelope.timestamp_ms);
                snapshot.latest_usage = Some(usage_snapshot);
            }
            Event::HistoryCompactionStarted { .. } => {
                let mut compaction = AppContextCompactionSnapshot::new("started");
                compaction.turn_id = envelope.turn_id;
                compaction.timestamp_ms = Some(envelope.timestamp_ms);
                snapshot.latest_compaction = Some(compaction);
            }
            Event::HistoryCompactionCompleted { report } => {
                let (report, summary_present) = sanitized_compaction_report(report);
                let mut compaction = AppContextCompactionSnapshot::new("completed");
                compaction.report = Some(report);
                compaction.summary_present = summary_present;
                compaction.turn_id = envelope.turn_id;
                compaction.timestamp_ms = Some(envelope.timestamp_ms);
                snapshot.latest_compaction = Some(compaction);
            }
            Event::HistoryCompactionFailed { .. } => {
                let mut compaction = AppContextCompactionSnapshot::new("failed");
                compaction.turn_id = envelope.turn_id;
                compaction.timestamp_ms = Some(envelope.timestamp_ms);
                snapshot.latest_compaction = Some(compaction);
            }
            _ => {}
        }
    }
}

fn token_usage_source_label(source: TokenUsageSource) -> &'static str {
    match source {
        TokenUsageSource::Estimated => "estimated",
        TokenUsageSource::Provider => "provider",
        TokenUsageSource::Mixed => "mixed",
        _ => "unknown",
    }
}

fn sanitized_compaction_report(
    report: &HistoryCompactionReport,
) -> (HistoryCompactionReport, bool) {
    let mut report = report.clone();
    let summary_present = report.summary.is_some();
    report.summary = None;
    report.metadata = Value::Null;
    (report, summary_present)
}

fn summarize_context_tools(events: &[EventEnvelope]) -> AppContextToolSummary {
    let mut names = BTreeSet::new();
    let mut summary = AppContextToolSummary::default();
    for envelope in events {
        match &envelope.event {
            Event::ToolCallRequested { call } => {
                summary.requested += 1;
                names.insert(call.name.clone());
            }
            Event::ToolFinished { result } => {
                summary.finished += 1;
                if !result.ok {
                    summary.failed += 1;
                }
            }
            _ => {}
        }
    }
    summary.names = names.into_iter().collect();
    summary
}

fn summarize_context_history(messages: &[CanonicalMessage]) -> AppContextHistorySummary {
    let mut summary = AppContextHistorySummary::default();
    summary.messages = messages.len();
    let mut bytes = 0usize;
    for message in messages {
        bytes += message.metadata.to_string().len() + 4;
        match message.role {
            MessageRole::User => summary.user_messages += 1,
            MessageRole::Assistant => summary.assistant_messages += 1,
            MessageRole::System | MessageRole::Developer => summary.system_messages += 1,
            MessageRole::Tool => {}
            _ => {}
        }
        for part in &message.parts {
            match part {
                ContentPart::Text { text }
                | ContentPart::ReasoningSummary { text }
                | ContentPart::Reasoning { text, .. } => {
                    bytes += text.len();
                }
                ContentPart::Context { chunk } => {
                    bytes += chunk.source.len()
                        + chunk
                            .path
                            .as_ref()
                            .map(|path| path.display().to_string().len())
                            .unwrap_or_default()
                        + chunk.content.len()
                        + chunk.metadata.to_string().len();
                }
                ContentPart::FileRef { path, content } => {
                    bytes += path.display().to_string().len()
                        + content.as_deref().unwrap_or_default().len();
                }
                ContentPart::ToolCall { call } => {
                    bytes += call.name.len() + call.args.to_string().len();
                }
                ContentPart::ToolResult { result } => {
                    summary.tool_results += 1;
                    bytes += result.output.len()
                        + result.error.as_deref().unwrap_or_default().len()
                        + result.metadata.to_string().len()
                        + result
                            .content
                            .iter()
                            .map(|content| {
                                serde_json::to_string(content)
                                    .map(|text| text.len())
                                    .unwrap_or(0)
                            })
                            .sum::<usize>();
                }
                ContentPart::Patch { patch } => {
                    bytes += patch.content.len();
                }
                _ => {}
            }
        }
    }
    summary.estimated_tokens = estimate_tokens_from_bytes(bytes);
    summary
}

fn estimate_tokens_from_bytes(bytes: usize) -> u32 {
    if bytes == 0 {
        0
    } else {
        u32::try_from((bytes / 4).max(1)).unwrap_or(u32::MAX)
    }
}
