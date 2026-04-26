use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::PatchApplier,
    domain::{Patch, PatchResult},
};

#[derive(Debug)]
pub struct DirectPatchApplier;

#[async_trait]
impl PatchApplier for DirectPatchApplier {
    async fn apply(&self, patch: Patch) -> Result<PatchResult> {
        Ok(PatchResult {
            ok: false,
            summary: format!(
                "direct patch applier stub received {} bytes",
                patch.content.len()
            ),
        })
    }
}
