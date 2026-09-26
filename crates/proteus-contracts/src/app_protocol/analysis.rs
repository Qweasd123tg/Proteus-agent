//! Read-only historical analysis projected from the canonical journal.
use serde::{Deserialize, Serialize};

use crate::{
    contracts::ModelCallOrigin,
    domain::{
        AgentOutput, AgentTask, ExchangeId, ExecutionId, HistoryCompactionReport, SessionId,
        ThreadId, ToolCall, ToolCallResolution, ToolResult, TurnId,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ModelFailure,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSessionAnalysis {
    pub session_id: SessionId,
    pub revision: u64,
    pub turns: Vec<AppAnalysisTurn>,
    pub selected: Option<AppTurnAnalysis>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppAnalysisTurn {
    pub turn_id: TurnId,
    pub number: usize,
    pub prompt_preview: String,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub status: AppAnalysisTurnStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppAnalysisTurnStatus {
    /// No settlement in this snapshot; does not imply a live running process.
    Unsettled,
    Success,
    Error,
    Canceled,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppTurnAnalysis {
    pub turn_id: TurnId,
    pub task: AgentTask,
    pub module_epoch: u64,
    /// The recorded, redacted configuration at turn admission, never current config.
    pub config_snapshot: serde_json::Value,
    pub steps: Vec<AppAnalysisStep>,
    pub output: Option<AgentOutput>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppAnalysisStep {
    /// Stable journal identity, independent of filtering and presentation order.
    pub sequence: u64,
    pub execution_id: Option<ExecutionId>,
    pub thread_id: Option<ThreadId>,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub data: AppAnalysisStepData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppAnalysisStepData {
    Model {
        exchange_id: ExchangeId,
        origin: ModelCallOrigin,
        request: Box<CanonicalModelRequest>,
        response: Option<Box<CanonicalModelResponse>>,
        failure: Option<ModelFailure>,
        /// Completed messages persisted before a possibly missing terminal response.
        messages: Vec<CanonicalMessage>,
    },
    Tool {
        call: ToolCall,
        approval_reason: Option<String>,
        resolution: Option<ToolCallResolution>,
        /// None means no recorded result, not that the external effect did not happen.
        result: Option<ToolResult>,
    },
    Compaction {
        report: HistoryCompactionReport,
        history_revision: u64,
    },
}
