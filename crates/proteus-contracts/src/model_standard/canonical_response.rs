use serde::{Deserialize, Serialize};

use crate::{domain::ToolCall, model_standard::CanonicalMessage};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_output_tokens: Option<u32>,
}

impl TokenUsage {
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
            reasoning_output_tokens: None,
        }
    }

    pub fn with_cached_input_tokens(mut self, tokens: Option<u32>) -> Self {
        self.cached_input_tokens = tokens;
        self
    }

    pub fn with_cache_creation_input_tokens(mut self, tokens: Option<u32>) -> Self {
        self.cache_creation_input_tokens = tokens;
        self
    }

    pub fn with_reasoning_output_tokens(mut self, tokens: Option<u32>) -> Self {
        self.reasoning_output_tokens = tokens;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_usage_accepts_legacy_json_without_details() {
        let usage: TokenUsage =
            serde_json::from_value(serde_json::json!({ "input_tokens": 10, "output_tokens": 2 }))
                .expect("legacy usage");

        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 2);
        assert_eq!(usage.cached_input_tokens, None);
        assert_eq!(usage.cache_creation_input_tokens, None);
        assert_eq!(usage.reasoning_output_tokens, None);
    }
}
