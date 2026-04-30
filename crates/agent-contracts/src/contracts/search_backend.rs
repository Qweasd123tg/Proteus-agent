use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::ContextChunk;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SearchQuery {
    pub text: String,
    pub cwd: PathBuf,
    pub max_results: usize,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>, cwd: PathBuf, max_results: usize) -> Self {
        Self {
            text: text.into(),
            cwd,
            max_results,
        }
    }
}

#[async_trait]
pub trait SearchBackend: Send + Sync {
    async fn search(&self, query: SearchQuery) -> Result<Vec<ContextChunk>>;
}
