use anyhow::Result;
use async_trait::async_trait;

use crate::{contracts::ContextBuildInput, domain::ContextChunk};

pub const BUILTIN_REPO_AWARE_PROVIDER_IDS: &[&str] = &[
    "project_instructions",
    "manifest",
    "git_status",
    "repo_tree",
    "memory",
    "search",
];

#[async_trait]
pub trait RepoAwareContextProvider: Send + Sync {
    async fn provide(&self, input: &ContextBuildInput) -> Result<Vec<ContextChunk>>;
}
