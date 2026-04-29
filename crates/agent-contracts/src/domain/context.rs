use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextChunk {
    pub source: String,
    pub path: Option<PathBuf>,
    pub content: String,
    pub score: Option<f32>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextBundle {
    pub chunks: Vec<ContextChunk>,
    pub summary: Option<String>,
    pub token_estimate: Option<u32>,
}
