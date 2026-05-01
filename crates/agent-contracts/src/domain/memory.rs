use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct MemoryItem {
    pub kind: String,
    pub content: String,
    pub metadata: serde_json::Value,
}

impl MemoryItem {
    pub fn new(
        kind: impl Into<String>,
        content: impl Into<String>,
        metadata: serde_json::Value,
    ) -> Self {
        Self {
            kind: kind.into(),
            content: content.into(),
            metadata,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct MemoryQuery {
    pub text: String,
    pub limit: usize,
}

impl MemoryQuery {
    pub fn new(text: impl Into<String>, limit: usize) -> Self {
        Self {
            text: text.into(),
            limit,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
#[non_exhaustive]
pub enum MemoryOp {
    Remember { item: MemoryItem },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[non_exhaustive]
pub struct MemoryPolicyPlan {
    #[serde(default)]
    pub ops: Vec<MemoryOp>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl MemoryPolicyPlan {
    pub fn new(ops: Vec<MemoryOp>) -> Self {
        Self {
            ops,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}
