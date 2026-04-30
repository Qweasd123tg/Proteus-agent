use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::{
    AgentOutput, AgentTask, CallId, EventId, ModelRef, PatchResult, SessionId, ThreadId, ToolCall,
    ToolResult, TurnId, new_event_id,
};
use crate::model_standard::FinishReason;

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
