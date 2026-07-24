use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    core::TurnSettlementStatus,
    domain::{AgentOutput, SessionId, ThreadId, TurnId},
};

pub const WORKFLOW_REPLAY_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkflowReplayOptions {
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowReplayReport {
    pub schema_version: u32,
    pub source: WorkflowReplaySource,
    pub recorded: WorkflowReplayOutcome,
    pub replay: WorkflowReplayOutcome,
    pub model_exchanges: WorkflowReplayCounts,
    pub tool_calls: WorkflowReplayCounts,
    pub comparison: WorkflowReplayComparison,
    pub source_journal_unchanged: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowReplaySource {
    pub journal_path: PathBuf,
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub module_epoch: u64,
    pub profile_name: String,
    pub workflow_id: String,
    pub policy_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowReplayOutcome {
    pub status: TurnSettlementStatus,
    pub output: Option<AgentOutput>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowReplayCounts {
    pub recorded: usize,
    pub replayed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowReplayComparison {
    pub matched: bool,
    pub settlement_equal: bool,
    pub output_equal: Option<bool>,
    pub error_equal: Option<bool>,
    pub history_equal: Option<bool>,
    pub issues: Vec<String>,
}
