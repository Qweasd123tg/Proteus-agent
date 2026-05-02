use anyhow::Result;
use async_trait::async_trait;

use crate::{contracts::ContextBuildInput, domain::ContextChunk};

#[async_trait]
pub trait RepoAwareContextProvider: Send + Sync {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>>;
}
