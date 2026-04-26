use serde::{Deserialize, Serialize};

use crate::domain::{CallId, ToolCall};
use crate::model_standard::{FinishReason, TokenUsage};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ModelStreamEvent {
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
