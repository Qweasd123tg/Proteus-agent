use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// How a model adapter presents a context chunk. Metadata never selects this mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextRenderMode {
    /// Prefix content with `Context from <source> (<path>):\n` (path is optional).
    SourceAnnotated,
    /// Send content exactly as provided, without an adapter-owned envelope.
    Verbatim,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ContextChunk {
    pub source: String,
    pub path: Option<PathBuf>,
    pub content: String,
    pub render_mode: ContextRenderMode,
    pub score: Option<f32>,
    pub metadata: serde_json::Value,
}

impl ContextChunk {
    pub fn new(source: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            path: None,
            content: content.into(),
            render_mode: ContextRenderMode::SourceAnnotated,
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

    pub fn with_render_mode(mut self, render_mode: ContextRenderMode) -> Self {
        self.render_mode = render_mode;
        self
    }
}

#[cfg(test)]
mod tests;

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
