use proteus_contracts::{
    domain::{
        AgentOutput, AgentTask, ExchangeId, HistoryCompactionReport, RecordId, SessionId, ThreadId,
        ToolCall, ToolCallResolution, ToolResult, TurnId,
    },
    model_standard::{CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse},
};
use serde::{Deserialize, Serialize};

pub const JOURNAL_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TurnOpened {
    pub task: AgentTask,
    pub base_history_revision: u64,
    pub module_epoch: u64,
    pub config_snapshot: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryMutationKind {
    Append,
    Replace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HistoryMutated {
    pub previous_revision: u64,
    pub new_revision: u64,
    pub mutation: HistoryMutationKind,
    pub messages: Vec<CanonicalMessage>,
    pub compaction: Option<HistoryCompactionReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelRequestRecorded {
    pub exchange_id: ExchangeId,
    pub request: CanonicalModelRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelResponseOutcome {
    Response { response: CanonicalModelResponse },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelResponseRecorded {
    pub exchange_id: ExchangeId,
    pub outcome: ModelResponseOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolCallRecordPhase {
    Requested,
    Resolved { resolution: ToolCallResolution },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolCallRecorded {
    pub call: ToolCall,
    pub phase: ToolCallRecordPhase,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolResultRecorded {
    pub result: ToolResult,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnSettlementStatus {
    Success,
    Error,
    Canceled,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TurnSettled {
    pub status: TurnSettlementStatus,
    pub output: Option<AgentOutput>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JournalKind {
    TurnOpened,
    HistoryMutated,
    ModelRequestRecorded,
    ModelResponseRecorded,
    ToolCallRecorded,
    ToolResultRecorded,
    TurnSettled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum JournalEntry {
    TurnOpened(TurnOpened),
    HistoryMutated(HistoryMutated),
    ModelRequestRecorded(ModelRequestRecorded),
    ModelResponseRecorded(ModelResponseRecorded),
    ToolCallRecorded(ToolCallRecorded),
    ToolResultRecorded(ToolResultRecorded),
    TurnSettled(TurnSettled),
}

impl JournalEntry {
    pub fn kind(&self) -> JournalKind {
        match self {
            Self::TurnOpened(_) => JournalKind::TurnOpened,
            Self::HistoryMutated(_) => JournalKind::HistoryMutated,
            Self::ModelRequestRecorded(_) => JournalKind::ModelRequestRecorded,
            Self::ModelResponseRecorded(_) => JournalKind::ModelResponseRecorded,
            Self::ToolCallRecorded(_) => JournalKind::ToolCallRecorded,
            Self::ToolResultRecorded(_) => JournalKind::ToolResultRecorded,
            Self::TurnSettled(_) => JournalKind::TurnSettled,
        }
    }

    pub(crate) fn payload_value(&self) -> serde_json::Result<serde_json::Value> {
        let mut value = serde_json::to_value(self)?;
        value
            .get_mut("payload")
            .map(serde_json::Value::take)
            .ok_or_else(|| serde::ser::Error::custom("journal entry omitted payload"))
    }

    pub(crate) fn from_kind_and_payload(
        kind: JournalKind,
        payload: serde_json::Value,
    ) -> serde_json::Result<Self> {
        serde_json::from_value(serde_json::json!({
            "kind": kind,
            "payload": payload,
        }))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JournalRecord {
    pub schema_version: u32,
    pub record_id: RecordId,
    pub session_seq: u64,
    pub timestamp_ms: i64,
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: Option<TurnId>,
    pub entry: JournalEntry,
}
