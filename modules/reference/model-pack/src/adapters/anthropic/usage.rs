use serde_json::Value;

use crate::model_standard::TokenUsage;

/// Anthropic reports cumulative counters in SSE events. A delta may omit
/// input/cache counters, so only fields present in that event replace state.
#[derive(Default)]
pub(super) struct AnthropicUsage {
    input: Option<u32>,
    output: Option<u32>,
    cache_creation: Option<u32>,
    cache_read: Option<u32>,
}

impl AnthropicUsage {
    pub(super) fn merge(&mut self, usage: &Value) {
        fn counter(usage: &Value, key: &str) -> Option<u32> {
            usage
                .get(key)
                .and_then(Value::as_u64)
                .map(|value| u32::try_from(value).unwrap_or(u32::MAX))
        }
        if let Some(value) = counter(usage, "input_tokens") {
            self.input = Some(value);
        }
        if let Some(value) = counter(usage, "output_tokens") {
            self.output = Some(value);
        }
        if let Some(value) = counter(usage, "cache_creation_input_tokens") {
            self.cache_creation = Some(value);
        }
        if let Some(value) = counter(usage, "cache_read_input_tokens") {
            self.cache_read = Some(value);
        }
    }

    pub(super) fn into_token_usage(self) -> Option<TokenUsage> {
        let input = self.input?;
        let output = self.output?;
        Some(
            TokenUsage::new(
                input
                    .saturating_add(self.cache_creation.unwrap_or(0))
                    .saturating_add(self.cache_read.unwrap_or(0)),
                output,
            )
            .with_cache_creation_input_tokens(self.cache_creation)
            .with_cached_input_tokens(self.cache_read),
        )
    }
}
