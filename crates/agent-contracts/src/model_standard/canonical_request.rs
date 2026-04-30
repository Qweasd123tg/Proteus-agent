use serde::{Deserialize, Serialize};

use crate::{
    domain::{
        CacheHints, ModelLimits, ModelRef, ReasoningConfig, ResponseFormat, SamplingConfig,
        ToolChoice, ToolSpec,
    },
    model_standard::CanonicalMessage,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct CanonicalModelRequest {
    pub model: ModelRef,
    pub instructions: Vec<InstructionBlock>,
    pub messages: Vec<CanonicalMessage>,
    pub tools: Vec<ToolSpec>,
    pub tool_choice: ToolChoice,
    pub response_format: ResponseFormat,
    pub sampling: SamplingConfig,
    pub reasoning: ReasoningConfig,
    pub limits: ModelLimits,
    pub cache: CacheHints,
    pub metadata: serde_json::Value,
}

impl CanonicalModelRequest {
    /// Минимальный конструктор с defaults. Остальные поля через `with_*`.
    pub fn new(model: ModelRef, messages: Vec<CanonicalMessage>) -> Self {
        Self {
            model,
            instructions: Vec::new(),
            messages,
            tools: Vec::new(),
            tool_choice: ToolChoice::default(),
            response_format: ResponseFormat::default(),
            sampling: SamplingConfig::default(),
            reasoning: ReasoningConfig::default(),
            limits: ModelLimits::default(),
            cache: CacheHints::default(),
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_instructions(mut self, instructions: Vec<InstructionBlock>) -> Self {
        self.instructions = instructions;
        self
    }
    pub fn with_tools(mut self, tools: Vec<ToolSpec>) -> Self {
        self.tools = tools;
        self
    }
    pub fn with_tool_choice(mut self, tool_choice: ToolChoice) -> Self {
        self.tool_choice = tool_choice;
        self
    }
    pub fn with_response_format(mut self, response_format: ResponseFormat) -> Self {
        self.response_format = response_format;
        self
    }
    pub fn with_sampling(mut self, sampling: SamplingConfig) -> Self {
        self.sampling = sampling;
        self
    }
    pub fn with_reasoning(mut self, reasoning: ReasoningConfig) -> Self {
        self.reasoning = reasoning;
        self
    }
    pub fn with_limits(mut self, limits: ModelLimits) -> Self {
        self.limits = limits;
        self
    }
    pub fn with_cache(mut self, cache: CacheHints) -> Self {
        self.cache = cache;
        self
    }
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct InstructionBlock {
    pub kind: InstructionKind,
    pub text: String,
    pub priority: u8,
}

impl InstructionBlock {
    pub fn new(kind: InstructionKind, text: impl Into<String>, priority: u8) -> Self {
        Self {
            kind,
            text: text.into(),
            priority,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum InstructionKind {
    System,
    Developer,
    Project,
    UserPreference,
}
