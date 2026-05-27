use anyhow::Result;
use async_trait::async_trait;

use crate::domain::{Patch, PatchResult};

#[async_trait]
pub trait PatchApplier: Send + Sync {
    async fn apply(&self, patch: Patch) -> Result<PatchResult>;
}
