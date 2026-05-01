use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::PatchApplier,
    domain::{Patch, PatchResult},
};

#[derive(Debug, Clone)]
pub struct NullPatchApplier;

#[async_trait]
impl PatchApplier for NullPatchApplier {
    async fn apply(&self, _patch: Patch) -> Result<PatchResult> {
        Ok(PatchResult::new(false, "patch applier is disabled"))
    }
}
