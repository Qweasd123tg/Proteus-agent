use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ContextChunk {
    pub source: String,
    pub path: Option<PathBuf>,
    pub content: String,
    pub score: Option<f32>,
    pub metadata: serde_json::Value,
}

impl ContextChunk {
    pub fn new(source: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            path: None,
            content: content.into(),
            score: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }

    pub fn with_score(mut self, score: f32) -> Self {
        self.score = Some(score);
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ContextBundle {
    pub chunks: Vec<ContextChunk>,
    pub summary: Option<String>,
    pub token_estimate: Option<u32>,
}

impl ContextBundle {
    pub fn new(chunks: Vec<ContextChunk>) -> Self {
        Self {
            chunks,
            summary: None,
            token_estimate: None,
        }
    }

    pub fn with_summary(mut self, summary: String) -> Self {
        self.summary = Some(summary);
        self
    }

    pub fn with_token_estimate(mut self, token_estimate: u32) -> Self {
        self.token_estimate = Some(token_estimate);
        self
    }
}
