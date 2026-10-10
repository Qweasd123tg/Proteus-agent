use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{ContextBuildInput, ProcessContextProviderInput},
    domain::ContextChunk,
};

#[async_trait]
pub trait RepoAwareContextProvider: Send + Sync {
    async fn provide(
        &self,
        input: ProcessContextProviderInput,
        context: &ContextBuildInput,
    ) -> Result<Vec<ContextChunk>>;
    async fn skill_catalog(
        &self,
        _cwd: &std::path::Path,
    ) -> Result<Option<crate::domain::SkillCatalog>> {
        Ok(None)
    }
}
