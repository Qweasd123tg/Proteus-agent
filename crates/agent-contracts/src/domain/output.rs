use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentOutput {
    pub text: String,
    pub metadata: serde_json::Value,
}
