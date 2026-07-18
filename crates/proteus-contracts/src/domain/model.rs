use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
}

impl ModelRef {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct SamplingConfig {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

impl SamplingConfig {
    pub fn new(temperature: Option<f32>, top_p: Option<f32>) -> Self {
        Self { temperature, top_p }
    }
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            temperature: Some(0.0),
            top_p: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ReasoningConfig {
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub summary: bool,
    #[serde(default)]
    pub budget_tokens: Option<u32>,
}

impl ReasoningConfig {
    pub fn new(effort: Option<String>, summary: bool) -> Self {
        Self {
            effort,
            summary,
            budget_tokens: None,
        }
    }

    pub fn with_budget_tokens(mut self, budget_tokens: Option<u32>) -> Self {
        self.budget_tokens = budget_tokens;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModelLimits {
    pub max_input_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
}

impl ModelLimits {
    pub fn new(max_input_tokens: Option<u32>, max_output_tokens: Option<u32>) -> Self {
        Self {
            max_input_tokens,
            max_output_tokens,
        }
    }
}

impl Default for ModelLimits {
    fn default() -> Self {
        Self {
            max_input_tokens: None,
            max_output_tokens: Some(16_384),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct CacheHints {
    pub cache_instructions: bool,
    pub cache_context: bool,
    /// Provider-neutral routing namespace for requests that share a stable
    /// prompt prefix. Adapters decide whether and how to map it to provider
    /// wire fields.
    pub routing_key: Option<String>,
}

impl CacheHints {
    pub fn new(cache_instructions: bool, cache_context: bool) -> Self {
        Self {
            cache_instructions,
            cache_context,
            routing_key: None,
        }
    }

    pub fn with_routing_key(mut self, routing_key: impl Into<String>) -> Self {
        self.routing_key = Some(routing_key.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ResponseFormat {
    #[default]
    Text,
    Json,
    JsonSchema {
        name: String,
        schema: serde_json::Value,
        strict: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ToolChoice {
    None,
    #[default]
    Auto,
    Required,
    Tool(String),
}
