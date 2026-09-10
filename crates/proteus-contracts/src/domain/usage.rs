//! Расход по сохранённым model exchanges, независимо от конкретного клиента.
use serde::{Deserialize, Serialize};

use crate::{
    contracts::ModelCallOrigin,
    domain::{ExchangeId, ModelRef, SessionId, TurnId},
    model_standard::{FinishReason, TokenUsage},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionUsageSnapshot {
    pub session_id: SessionId,
    pub revision: u64,
    pub latest_turn_id: Option<TurnId>,
    /// Порядок начала запросов. Повторы и compactor exchanges — отдельные строки.
    pub requests: Vec<ModelRequestUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelRequestUsage {
    pub exchange_id: ExchangeId,
    pub turn_id: Option<TurnId>,
    pub model: ModelRef,
    pub origin: ModelCallOrigin,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub status: ModelUsageStatus,
    pub finish_reason: Option<FinishReason>,
    /// Только provider usage. Отсутствие не означает нулевой расход.
    pub usage: Option<TokenUsage>,
    pub message_count: usize,
    pub tool_count: usize,
    pub reasoning_effort: Option<String>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelUsageStatus {
    Unfinished,
    Completed,
    Error,
    Canceled,
    Timeout,
}
