use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::domain::{CallId, Event, EventEnvelope, ToolCall, TurnId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalReport {
    pub event_log_path: PathBuf,
    pub events: usize,
    pub turns_started: usize,
    pub turns_finished: usize,
    pub turns_failed: usize,
    pub model_calls: usize,
    pub tool_calls: usize,
    pub tool_failures: usize,
    pub approvals_requested: usize,
    pub approvals_resolved: usize,
    pub approvals_approved: usize,
    pub approvals_denied: usize,
    pub estimated_input_tokens: u64,
    pub provider_input_tokens: u64,
    pub provider_output_tokens: u64,
    pub changed_files: Vec<String>,
    pub duration_ms: Option<u64>,
    pub failure_reason: Option<String>,
}

impl EvalReport {
    pub fn succeeded(&self) -> bool {
        self.turns_started > 0
            && self.turns_finished > 0
            && self.turns_failed == 0
            && self.failure_reason.is_none()
    }
}

#[derive(Debug, Default)]
struct TurnStats {
    finished: bool,
    failed: bool,
}

#[derive(Debug, Default)]
struct EvalAccumulator {
    events: usize,
    turns: BTreeMap<TurnId, TurnStats>,
    first_timestamp_ms: Option<i64>,
    last_timestamp_ms: Option<i64>,
    model_calls: usize,
    tool_calls: usize,
    tool_failures: usize,
    approvals_requested: usize,
    approvals_resolved: usize,
    approvals_approved: usize,
    approvals_denied: usize,
    estimated_input_tokens: u64,
    provider_input_tokens: u64,
    provider_output_tokens: u64,
    changed_files: BTreeSet<String>,
    failure_reason: Option<String>,
    calls: BTreeMap<CallId, ToolCall>,
}

pub fn read_eval_report(path: impl AsRef<Path>) -> Result<EvalReport> {
    let path = path.as_ref();
    let file =
        File::open(path).with_context(|| format!("failed to open event log {}", path.display()))?;
    let mut accumulator = EvalAccumulator::default();

    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.with_context(|| format!("failed to read event log line {}", index + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let envelope: EventEnvelope = serde_json::from_str(&line)
            .with_context(|| format!("failed to parse event log line {}", index + 1))?;
        accumulator.record(envelope);
    }

    Ok(accumulator.finish(path.to_path_buf()))
}

impl EvalAccumulator {
    fn record(&mut self, envelope: EventEnvelope) {
        self.events += 1;
        self.first_timestamp_ms = Some(
            self.first_timestamp_ms
                .map_or(envelope.timestamp_ms, |seen| {
                    seen.min(envelope.timestamp_ms)
                }),
        );
        self.last_timestamp_ms = Some(
            self.last_timestamp_ms
                .map_or(envelope.timestamp_ms, |seen| {
                    seen.max(envelope.timestamp_ms)
                }),
        );

        match envelope.event {
            Event::TurnStarted { turn_id, .. } => {
                self.turns.entry(turn_id).or_default();
            }
            Event::ModelRequestPrepared { .. } => {
                self.model_calls += 1;
            }
            Event::TokenUsageUpdated { usage } => {
                self.estimated_input_tokens += u64::from(usage.estimated_input_tokens);
                if let Some(actual) = usage.actual {
                    self.provider_input_tokens += u64::from(actual.input_tokens);
                    self.provider_output_tokens += u64::from(actual.output_tokens);
                }
            }
            Event::ToolCallRequested { call } => {
                self.tool_calls += 1;
                self.calls.insert(call.id.clone(), call);
            }
            Event::ApprovalRequested { .. } => {
                self.approvals_requested += 1;
            }
            Event::ApprovalResolved { approved, .. } => {
                self.approvals_resolved += 1;
                if approved {
                    self.approvals_approved += 1;
                } else {
                    self.approvals_denied += 1;
                }
            }
            Event::ToolFinished { result } => {
                if !result.ok {
                    self.tool_failures += 1;
                }
                if result.ok
                    && let Some(call) = self.calls.get(&result.call_id)
                {
                    record_changed_files(&mut self.changed_files, call, &result.metadata);
                }
            }
            Event::Error { message } => {
                if let Some(turn_id) = envelope.turn_id {
                    self.turns.entry(turn_id).or_default().failed = true;
                }
                if self.failure_reason.is_none() {
                    self.failure_reason = Some(message);
                }
            }
            Event::TurnFinished { .. } => {
                if let Some(turn_id) = envelope.turn_id {
                    self.turns.entry(turn_id).or_default().finished = true;
                }
            }
            _ => {}
        }
    }

    fn finish(mut self, event_log_path: PathBuf) -> EvalReport {
        let mut unfinished = 0;
        for turn in self.turns.values_mut() {
            if !turn.finished {
                turn.failed = true;
                unfinished += 1;
            }
        }
        if unfinished > 0 && self.failure_reason.is_none() {
            self.failure_reason = Some(format!("{unfinished} unfinished turn(s)"));
        }

        let turns_started = self.turns.len();
        let turns_finished = self.turns.values().filter(|turn| turn.finished).count();
        let turns_failed = self.turns.values().filter(|turn| turn.failed).count();
        let duration_ms = match (self.first_timestamp_ms, self.last_timestamp_ms) {
            (Some(first), Some(last)) if last >= first => Some((last - first) as u64),
            _ => None,
        };

        EvalReport {
            event_log_path,
            events: self.events,
            turns_started,
            turns_finished,
            turns_failed,
            model_calls: self.model_calls,
            tool_calls: self.tool_calls,
            tool_failures: self.tool_failures,
            approvals_requested: self.approvals_requested,
            approvals_resolved: self.approvals_resolved,
            approvals_approved: self.approvals_approved,
            approvals_denied: self.approvals_denied,
            estimated_input_tokens: self.estimated_input_tokens,
            provider_input_tokens: self.provider_input_tokens,
            provider_output_tokens: self.provider_output_tokens,
            changed_files: self.changed_files.into_iter().collect(),
            duration_ms,
            failure_reason: self.failure_reason,
        }
    }
}

fn record_changed_files(
    changed_files: &mut BTreeSet<String>,
    call: &ToolCall,
    metadata: &serde_json::Value,
) {
    match call.name.as_str() {
        "write_file" => {
            if let Some(path) = metadata
                .get("path")
                .and_then(serde_json::Value::as_str)
                .or_else(|| call.args.get("path").and_then(serde_json::Value::as_str))
            {
                changed_files.insert(path.to_owned());
            }
        }
        "apply_patch" => {
            if let Some(patch) = call.args.get("patch").and_then(serde_json::Value::as_str) {
                for path in patch_paths(patch) {
                    changed_files.insert(path);
                }
            }
        }
        _ => {}
    }
}

fn patch_paths(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|line| {
            line.strip_prefix("*** Add File: ")
                .or_else(|| line.strip_prefix("*** Update File: "))
                .or_else(|| line.strip_prefix("*** Delete File: "))
                .or_else(|| line.strip_prefix("*** Move to: "))
        })
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use crate::{
        domain::{
            AgentOutput, EventContext, ModelRef, TokenUsageCategory, TokenUsageSnapshot,
            ToolResult, new_call_id, new_session_id, new_thread_id, new_turn_id,
        },
        model_standard::TokenUsage,
    };
    use serde_json::json;
    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn report_summarizes_successful_turn() {
        let session_id = new_session_id();
        let thread_id = new_thread_id();
        let turn_id = new_turn_id();
        let call_id = new_call_id();
        let mut log = NamedTempFile::new().expect("event log");

        write_event(
            &mut log,
            1,
            EventContext::new(session_id.clone(), thread_id.clone(), Some(turn_id.clone())),
            Event::TurnStarted {
                session_id: session_id.clone(),
                thread_id,
                turn_id: turn_id.clone(),
            },
        );
        write_event(
            &mut log,
            2,
            EventContext::new(session_id.clone(), new_thread_id(), Some(turn_id.clone())),
            Event::ModelRequestPrepared {
                model: ModelRef::new("fake", "test"),
            },
        );
        write_event(
            &mut log,
            3,
            EventContext::new(session_id.clone(), new_thread_id(), Some(turn_id.clone())),
            Event::TokenUsageUpdated {
                usage: TokenUsageSnapshot::new(
                    ModelRef::new("fake", "test"),
                    11,
                    vec![TokenUsageCategory::new("messages", 11)],
                )
                .with_actual(Some(TokenUsage::new(13, 5))),
            },
        );
        write_event(
            &mut log,
            4,
            EventContext::new(session_id.clone(), new_thread_id(), Some(turn_id.clone())),
            Event::ToolCallRequested {
                call: ToolCall::new(
                    call_id.clone(),
                    "apply_patch",
                    json!({ "patch": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch" }),
                ),
            },
        );
        write_event(
            &mut log,
            5,
            EventContext::new(session_id.clone(), new_thread_id(), Some(turn_id.clone())),
            Event::ToolFinished {
                result: ToolResult::ok(call_id, "updated src/lib.rs"),
            },
        );
        write_event(
            &mut log,
            6,
            EventContext::new(session_id, new_thread_id(), Some(turn_id)),
            Event::TurnFinished {
                output: AgentOutput::text("done"),
            },
        );

        let report = read_eval_report(log.path()).expect("report");
        assert!(report.succeeded());
        assert_eq!(report.events, 6);
        assert_eq!(report.turns_started, 1);
        assert_eq!(report.turns_finished, 1);
        assert_eq!(report.turns_failed, 0);
        assert_eq!(report.model_calls, 1);
        assert_eq!(report.tool_calls, 1);
        assert_eq!(report.estimated_input_tokens, 11);
        assert_eq!(report.provider_input_tokens, 13);
        assert_eq!(report.provider_output_tokens, 5);
        assert_eq!(report.changed_files, vec!["src/lib.rs"]);
        assert_eq!(report.duration_ms, Some(5));
    }

    #[test]
    fn report_marks_unfinished_turn_failed() {
        let session_id = new_session_id();
        let thread_id = new_thread_id();
        let turn_id = new_turn_id();
        let mut log = NamedTempFile::new().expect("event log");

        write_event(
            &mut log,
            10,
            EventContext::new(session_id.clone(), thread_id.clone(), Some(turn_id.clone())),
            Event::TurnStarted {
                session_id,
                thread_id,
                turn_id,
            },
        );

        let report = read_eval_report(log.path()).expect("report");
        assert!(!report.succeeded());
        assert_eq!(report.turns_failed, 1);
        assert_eq!(
            report.failure_reason.as_deref(),
            Some("1 unfinished turn(s)")
        );
    }

    #[test]
    fn report_error_marks_status_failed_even_after_finished_turn() {
        let session_id = new_session_id();
        let thread_id = new_thread_id();
        let turn_id = new_turn_id();
        let mut log = NamedTempFile::new().expect("event log");

        write_event(
            &mut log,
            1,
            EventContext::new(session_id.clone(), thread_id.clone(), Some(turn_id.clone())),
            Event::TurnStarted {
                session_id: session_id.clone(),
                thread_id,
                turn_id: turn_id.clone(),
            },
        );
        write_event(
            &mut log,
            2,
            EventContext::new(session_id.clone(), new_thread_id(), Some(turn_id.clone())),
            Event::TurnFinished {
                output: AgentOutput::text("done"),
            },
        );
        write_event(
            &mut log,
            3,
            EventContext::new(session_id, new_thread_id(), None),
            Event::Error {
                message: "transport failed".to_owned(),
            },
        );

        let report = read_eval_report(log.path()).expect("report");
        assert!(!report.succeeded());
        assert_eq!(report.failure_reason.as_deref(), Some("transport failed"));
    }

    #[test]
    fn patch_paths_extracts_edit_headers() {
        assert_eq!(
            patch_paths(
                "*** Begin Patch\n*** Add File: a.txt\n+x\n*** Update File: b.txt\n@@\n-y\n+z\n*** Move to: c.txt\n*** Delete File: d.txt\n*** End Patch"
            ),
            vec!["a.txt", "b.txt", "c.txt", "d.txt"]
        );
    }

    fn write_event(
        log: &mut NamedTempFile,
        timestamp_ms: i64,
        context: EventContext,
        event: Event,
    ) {
        let mut envelope = EventEnvelope::new(context, timestamp_ms as u64, event);
        envelope.timestamp_ms = timestamp_ms;
        serde_json::to_writer(&mut *log, &envelope).expect("write event");
        writeln!(log).expect("newline");
    }
}
