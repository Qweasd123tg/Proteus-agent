use serde::{Deserialize, Serialize};

use crate::domain::{CallId, ToolCall};
use crate::model_standard::{CanonicalModelResponse, FinishReason, TokenUsage};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum ModelStreamEvent {
    Response {
        response: CanonicalModelResponse,
    },
    TextDelta {
        text: String,
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
