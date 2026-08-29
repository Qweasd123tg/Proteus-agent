use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    core::{
        JournalEntry, JournalRecord, ModelResponseOutcome, SessionStore, ToolCallRecordPhase,
        TurnSettlementStatus, normalize_session_dir_path,
    },
    domain::{CallId, ToolCall, ToolCallResolution, TurnId},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalReport {
    pub journal_path: PathBuf,
    pub records: usize,
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
            && self.turns_finished == self.turns_started
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
    records: usize,
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
    let session_dir = normalize_session_dir_path(path.as_ref().to_path_buf())?;
    let store = SessionStore::open(session_dir.clone()).with_context(|| {
        format!(
            "failed to open canonical session journal at {}",
            session_dir.display()
        )
    })?;
    // Projection validation is the corruption/linkage gate shared with resume.
    let projection = store.load_projection()?;
    let mut accumulator = EvalAccumulator::default();
    for record in &projection.records {
        accumulator.record(record)?;
    }
    Ok(accumulator.finish(store.journal_path()))
}

impl EvalAccumulator {
    fn record(&mut self, record: &JournalRecord) -> Result<()> {
        self.records += 1;
        self.first_timestamp_ms = Some(
            self.first_timestamp_ms
                .map_or(record.timestamp_ms, |seen| seen.min(record.timestamp_ms)),
        );
        self.last_timestamp_ms = Some(
            self.last_timestamp_ms
                .map_or(record.timestamp_ms, |seen| seen.max(record.timestamp_ms)),
        );

        match &record.entry {
            JournalEntry::TurnOpened(_) => {
                if let Some(turn_id) = record.turn_id {
                    self.turns.entry(turn_id).or_default();
                }
            }
            JournalEntry::ModelRequestRecorded(model) => {
                self.model_calls += 1;
                let bytes = serde_json::to_vec(&model.request)?.len();
                self.estimated_input_tokens = self
                    .estimated_input_tokens
                    .saturating_add((bytes / 4).max(1) as u64);
            }
            JournalEntry::ModelResponseRecorded(model) => {
                if let ModelResponseOutcome::Response { response } = &model.outcome
                    && let Some(usage) = &response.usage
                {
                    self.provider_input_tokens = self
                        .provider_input_tokens
                        .saturating_add(u64::from(usage.input_tokens));
                    self.provider_output_tokens = self
                        .provider_output_tokens
                        .saturating_add(u64::from(usage.output_tokens));
                }
            }
            JournalEntry::ToolCallRecorded(tool) => match &tool.phase {
                ToolCallRecordPhase::Requested => {
                    self.tool_calls += 1;
                    self.calls.insert(tool.call.id.clone(), tool.call.clone());
                }
                ToolCallRecordPhase::ApprovalRequested { .. } => {
                    self.approvals_requested += 1;
                }
                ToolCallRecordPhase::Resolved { resolution } => {
                    if resolution.requested_approval() {
                        self.approvals_resolved += 1;
                        match resolution {
                            ToolCallResolution::Approved => self.approvals_approved += 1,
                            ToolCallResolution::ApprovalDenied { .. } => self.approvals_denied += 1,
                            _ => {}
                        }
                    }
                }
            },
            JournalEntry::ToolResultRecorded(tool) => {
                if !tool.result.ok {
                    self.tool_failures += 1;
                }
                if tool.result.ok
                    && let Some(call) = self.calls.get(&tool.result.call_id)
                {
                    record_changed_files(&mut self.changed_files, call, &tool.result.metadata);
                }
            }
            JournalEntry::TurnSettled(settled) => {
                if let Some(turn_id) = record.turn_id {
                    let turn = self.turns.entry(turn_id).or_default();
                    turn.finished = true;
                    turn.failed = settled.status != TurnSettlementStatus::Success;
                }
                if settled.status != TurnSettlementStatus::Success && self.failure_reason.is_none()
                {
                    self.failure_reason = settled
                        .error
                        .clone()
                        .or_else(|| Some(format!("turn settled with status {:?}", settled.status)));
                }
            }
            JournalEntry::HistoryMutated(_) => {}
        }
        Ok(())
    }

    fn finish(mut self, journal_path: PathBuf) -> EvalReport {
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
            journal_path,
            records: self.records,
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
    use serde_json::json;

    use super::*;
    use crate::{
        contracts::ExecutionAttribution,
        core::{
            ModelRequestRecorded, ModelResponseRecorded, ToolCallRecorded, ToolResultRecorded,
            TurnOpened, TurnSettled,
        },
        domain::{
            AgentOutput, AgentTask, ModelRef, ToolCallResolution, ToolResult, new_call_id,
            new_exchange_id, new_execution_id, new_session_id, new_thread_id, new_turn_id,
        },
        model_standard::{
            CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, FinishReason,
            MessageRole, TokenUsage,
        },
    };

    #[tokio::test]
    async fn report_uses_canonical_journal_records() {
        let config_dir = tempfile::tempdir().expect("config");
        let workspace = tempfile::tempdir().expect("workspace");
        let store = SessionStore::new(config_dir.path(), workspace.path(), new_session_id())
            .expect("store");
        let thread_id = new_thread_id();
        let turn_id = new_turn_id();
        let attribution = ExecutionAttribution::for_turn(
            new_execution_id(),
            store.session_id(),
            thread_id,
            turn_id,
        );
        store
            .append_execution_journal_entry(
                attribution,
                JournalEntry::TurnOpened(TurnOpened {
                    task: AgentTask::new("edit", workspace.path().to_path_buf()),
                    base_history_revision: 0,
                    module_epoch: 0,
                    config_snapshot: json!({}),
                }),
            )
            .await
            .expect("turn opened");
        let exchange_id = new_exchange_id();
        let request = CanonicalModelRequest::new(
            ModelRef::new("fake", "model"),
            vec![CanonicalMessage::text(MessageRole::User, "edit")],
        );
        store
            .append_execution_journal_entry(
                attribution,
                JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                    exchange_id,
                    request,
                }),
            )
            .await
            .expect("model request");
        let response = CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "done"),
            Vec::new(),
            FinishReason::Stop,
        )
        .with_usage(TokenUsage::new(120, 30));
        store
            .append_execution_journal_entry(
                attribution,
                JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                    exchange_id,
                    outcome: ModelResponseOutcome::Response { response },
                }),
            )
            .await
            .expect("model response");
        let call = ToolCall::new(
            new_call_id(),
            "write_file",
            json!({ "path": "src/output.rs" }),
        );
        for phase in [
            ToolCallRecordPhase::Requested,
            ToolCallRecordPhase::ApprovalRequested {
                reason: "write".to_owned(),
            },
            ToolCallRecordPhase::Resolved {
                resolution: ToolCallResolution::Approved,
            },
        ] {
            store
                .append_execution_journal_entry(
                    attribution,
                    JournalEntry::ToolCallRecorded(ToolCallRecorded {
                        call: call.clone(),
                        phase,
                    }),
                )
                .await
                .expect("tool call phase");
        }
        store
            .append_execution_journal_entry(
                attribution,
                JournalEntry::ToolResultRecorded(ToolResultRecorded {
                    result: ToolResult::ok(call.id, "written"),
                }),
            )
            .await
            .expect("tool result");
        store
            .append_journal_entry(
                thread_id,
                Some(turn_id),
                JournalEntry::TurnSettled(TurnSettled {
                    status: TurnSettlementStatus::Success,
                    output: Some(AgentOutput::text("done")),
                    error: None,
                }),
            )
            .await
            .expect("turn settled");

        let report = read_eval_report(store.session_dir()).expect("report");

        assert!(report.succeeded());
        assert_eq!(report.records, 8);
        assert_eq!(report.turns_started, 1);
        assert_eq!(report.turns_finished, 1);
        assert_eq!(report.model_calls, 1);
        assert_eq!(report.tool_calls, 1);
        assert_eq!(report.approvals_requested, 1);
        assert_eq!(report.approvals_resolved, 1);
        assert_eq!(report.approvals_approved, 1);
        assert_eq!(report.provider_input_tokens, 120);
        assert_eq!(report.provider_output_tokens, 30);
        assert_eq!(report.changed_files, vec!["src/output.rs"]);
        assert_eq!(report.journal_path, store.journal_path());
    }

    #[tokio::test]
    async fn unfinished_turn_is_reported_as_failure() {
        let config_dir = tempfile::tempdir().expect("config");
        let workspace = tempfile::tempdir().expect("workspace");
        let store = SessionStore::new(config_dir.path(), workspace.path(), new_session_id())
            .expect("store");
        let turn_id = new_turn_id();
        let thread_id = new_thread_id();
        let attribution = ExecutionAttribution::for_turn(
            new_execution_id(),
            store.session_id(),
            thread_id,
            turn_id,
        );
        store
            .append_execution_journal_entry(
                attribution,
                JournalEntry::TurnOpened(TurnOpened {
                    task: AgentTask::new("crash", workspace.path().to_path_buf()),
                    base_history_revision: 0,
                    module_epoch: 0,
                    config_snapshot: json!({}),
                }),
            )
            .await
            .expect("turn opened");

        let report = read_eval_report(store.journal_path()).expect("report");

        assert!(!report.succeeded());
        assert_eq!(report.turns_failed, 1);
        assert_eq!(
            report.failure_reason.as_deref(),
            Some("1 unfinished turn(s)")
        );
    }
}
