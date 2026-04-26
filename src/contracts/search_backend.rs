use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;

use crate::domain::ContextChunk;

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub cwd: PathBuf,
    pub max_results: usize,
}

#[async_trait]
pub trait SearchBackend: Send + Sync {
    async fn search(&self, query: SearchQuery) -> Result<Vec<ContextChunk>>;
}
