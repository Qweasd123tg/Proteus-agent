use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct AgentOutput {
    pub text: String,
    pub metadata: serde_json::Value,
}

impl AgentOutput {
    pub fn new(text: impl Into<String>, metadata: serde_json::Value) -> Self {
        Self {
            text: text.into(),
            metadata,
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            metadata: serde_json::Value::Null,
        }
    }
}
