use serde::{Deserialize, Serialize};

use crate::{
    domain::{
        CacheHints, ModelLimits, ModelRef, ReasoningConfig, ResponseFormat, SamplingConfig,
        ToolChoice, ToolSpec,
    },
    model_standard::CanonicalMessage,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
