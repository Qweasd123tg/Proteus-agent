use serde::{Deserialize, Serialize};

use crate::domain::{CallId, MessageId, ToolCall};
use crate::model_standard::{
    CanonicalMessage, CanonicalModelResponse, FinishReason, MessagePhase, TokenUsage,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub enum ModelStreamEvent {
    Response {
        response: CanonicalModelResponse,
    },
    TextDelta {
        message_id: MessageId,
        phase: Option<MessagePhase>,
        text: String,
    },
    /// Authoritative completed assistant item, including tool calls. The terminal
    /// Response retains the same id and content; a failure retains it in
    /// completed_messages. Completed items form an ordered prefix of Response.
    /// Adapters may omit this event entirely, or stop emitting it at a prefix
    /// boundary, but must not skip an earlier output item then emit a later one.
    MessageCompleted {
        message: CanonicalMessage,
    },
    ToolCallDelta {
        call_id: CallId,
        name: Option<String>,
        args_delta: String,
    },
    ToolCallFinished {
        call: ToolCall,
    },
    ReasoningSummaryDelta {
        text: String,
    },
    Usage {
        usage: TokenUsage,
    },
    Done {
        finish_reason: FinishReason,
    },
    Error {
        failure: crate::model_standard::ModelFailure,
    },
}
