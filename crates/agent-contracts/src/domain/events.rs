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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct TokenUsageSnapshot {
    pub model: ModelRef,
    pub phase: Option<String>,
    pub estimated_input_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    pub categories: Vec<TokenUsageCategory>,
    pub actual: Option<TokenUsage>,
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
            categories,
            actual: None,
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

    pub fn with_max_input_tokens(mut self, max_input_tokens: Option<u32>) -> Self {
        self.max_input_tokens = max_input_tokens;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum Event {
    SessionStarted {
        session_id: SessionId,
        cwd: PathBuf,
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
