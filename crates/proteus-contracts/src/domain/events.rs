use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::{
    AgentOutput, AgentTask, CallId, EventId, ModelRef, PatchResult, SessionId, ThreadId, ToolCall,
    ToolResult, TurnId, new_event_id,
};
use crate::model_standard::{FinishReason, TokenUsage};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct EventContext {
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: Option<TurnId>,
}

impl EventContext {
    pub fn new(session_id: SessionId, thread_id: ThreadId, turn_id: Option<TurnId>) -> Self {
        Self {
            session_id,
            thread_id,
            turn_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct EventEnvelope {
    pub schema_version: u32,
    pub event_id: EventId,
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: Option<TurnId>,
    pub seq: u64,
    pub timestamp_ms: i64,
    pub event: Event,
}

impl EventEnvelope {
    pub fn new(context: EventContext, seq: u64, event: Event) -> Self {
        Self {
            schema_version: 1,
            event_id: new_event_id(),
            session_id: context.session_id,
            thread_id: context.thread_id,
            turn_id: context.turn_id,
            seq,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            event,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct TokenUsageCategory {
    pub name: String,
    pub tokens: u32,
}

impl TokenUsageCategory {
    pub fn new(name: impl Into<String>, tokens: u32) -> Self {
        Self {
            name: name.into(),
            tokens,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TokenUsageSource {
    /// Local estimate only. Good for attribution, not billing truth.
    Estimated,
    /// Provider-reported totals only.
    Provider,
    /// Provider totals plus local category estimates.
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct TokenUsageSnapshot {
    pub model: ModelRef,
    pub phase: Option<String>,
    pub estimated_input_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    /// Оценка порога входных токенов, на котором workflow запускает
    /// автокомпакт истории. Питает метку на индикаторе контекста в клиентах.
    /// `None`, если автокомпакт не настроен или потолок окна неизвестен.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_trigger_tokens: Option<u32>,
    pub categories: Vec<TokenUsageCategory>,
    pub actual: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<TokenUsageSource>,
}

impl TokenUsageSnapshot {
    pub fn new(
        model: ModelRef,
        estimated_input_tokens: u32,
        categories: Vec<TokenUsageCategory>,
    ) -> Self {
        Self {
            model,
            phase: None,
            estimated_input_tokens,
            max_input_tokens: None,
            compaction_trigger_tokens: None,
            categories,
            actual: None,
            source: None,
        }
    }

    pub fn with_phase(mut self, phase: impl Into<String>) -> Self {
        self.phase = Some(phase.into());
        self
    }

    pub fn with_actual(mut self, actual: Option<TokenUsage>) -> Self {
        self.actual = actual;
        self
    }

    pub fn with_source(mut self, source: TokenUsageSource) -> Self {
        self.source = Some(source);
        self
    }

    pub fn usage_source(&self) -> TokenUsageSource {
        self.source.unwrap_or_else(|| {
            if self.actual.is_some() && !self.categories.is_empty() {
                TokenUsageSource::Mixed
            } else if self.actual.is_some() {
                TokenUsageSource::Provider
            } else {
                TokenUsageSource::Estimated
            }
        })
    }

    pub fn with_max_input_tokens(mut self, max_input_tokens: Option<u32>) -> Self {
        self.max_input_tokens = max_input_tokens;
        self
    }

    pub fn with_compaction_trigger_tokens(mut self, trigger_tokens: Option<u32>) -> Self {
        self.compaction_trigger_tokens = trigger_tokens;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct HistoryCompactionReport {
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub input_messages: usize,
    pub output_messages: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_token_estimate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_token_estimate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl HistoryCompactionReport {
    pub fn unchanged(input_messages: usize, reason: Option<String>) -> Self {
        Self {
            changed: false,
            reason,
            input_messages,
            output_messages: input_messages,
            original_token_estimate: None,
            output_token_estimate: None,
            trigger_tokens: None,
            summary_source: None,
            skipped_reason: None,
            summary: None,
            metadata: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum Event {
    SessionStarted {
        session_id: SessionId,
        cwd: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<ModelRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_dir: Option<PathBuf>,
    },
    TurnStarted {
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
    },
    TaskReceived {
        task: AgentTask,
    },
    ContextBuilt {
        chunks: usize,
        token_estimate: Option<u32>,
    },
    ModelRequestPrepared {
        model: ModelRef,
    },
    ModelResponseReceived {
        finish_reason: FinishReason,
    },
    TokenUsageUpdated {
        usage: TokenUsageSnapshot,
    },
    HistoryCompactionStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        input_messages: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_estimate: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger_tokens: Option<u32>,
    },
    HistoryCompactionCompleted {
        report: HistoryCompactionReport,
    },
    HistoryCompactionFailed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        input_messages: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_estimate: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger_tokens: Option<u32>,
        message: String,
    },
    /// Частичный текстовый chunk от модели во время стрима. Эмитится по
    /// мере прихода SSE-event'ов из провайдера, до финального
    /// `ModelResponseReceived`. UI использует для in-place append;
    /// persistence по умолчанию пропускает (см. `FilteredEventSink`).
    AssistantTextDelta {
        text: String,
    },
    /// Частичные аргументы tool call'а от модели. Приходит построчно
    /// через SSE, дополняет `AssistantTextDelta` при stream'е ответа
    /// со смешанным содержимым.
    AssistantToolArgsDelta {
        call_id: CallId,
        args_delta: String,
    },
    /// Частичное reasoning-summary (только OpenAI o-series), plain text.
    /// Anthropic этого не шлёт, событие будет отсутствовать для их
    /// ответов.
    AssistantReasoningDelta {
        text: String,
    },
    ToolCallRequested {
        call: ToolCall,
    },
    ApprovalRequested {
        call_id: CallId,
        reason: String,
    },
    ApprovalResolved {
        call_id: CallId,
        approved: bool,
    },
    ToolFinished {
        result: ToolResult,
    },
    MemoryWritten {
        kind: String,
    },
    PatchApplied {
        result: PatchResult,
    },
    TurnFinished {
        output: AgentOutput,
    },
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_usage_snapshot_infers_source_for_legacy_events() {
        let estimated = TokenUsageSnapshot::new(
            ModelRef::new("test", "model"),
            10,
            vec![TokenUsageCategory::new("messages", 10)],
        );
        assert_eq!(estimated.usage_source(), TokenUsageSource::Estimated);

        let mixed = TokenUsageSnapshot::new(
            ModelRef::new("test", "model"),
            10,
            vec![TokenUsageCategory::new("messages", 10)],
        )
        .with_actual(Some(TokenUsage::new(11, 2)));
        assert_eq!(mixed.usage_source(), TokenUsageSource::Mixed);

        let provider = TokenUsageSnapshot::new(ModelRef::new("test", "model"), 0, Vec::new())
            .with_actual(Some(TokenUsage::new(11, 2)));
        assert_eq!(provider.usage_source(), TokenUsageSource::Provider);
    }
}
