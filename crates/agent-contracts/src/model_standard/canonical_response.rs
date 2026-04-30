use serde::{Deserialize, Serialize};

use crate::{domain::ToolCall, model_standard::CanonicalMessage};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalModelResponse {
    pub message: CanonicalMessage,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub usage: Option<TokenUsage>,
    pub provider_metadata: serde_json::Value,
}

impl CanonicalModelResponse {
    pub fn new(
        message: CanonicalMessage,
        tool_calls: Vec<ToolCall>,
        finish_reason: FinishReason,
    ) -> Self {
        Self {
            message,
            tool_calls,
            finish_reason,
            usage: None,
            provider_metadata: serde_json::Value::Null,
        }
    }

    pub fn with_usage(mut self, usage: TokenUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_provider_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.provider_metadata = metadata;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl TokenUsage {
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
        }
    }
}
