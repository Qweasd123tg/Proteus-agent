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
