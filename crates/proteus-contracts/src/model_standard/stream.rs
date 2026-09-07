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
    /// Authoritative completed item; the terminal Response retains the same id.
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
        message: String,
    },
}
