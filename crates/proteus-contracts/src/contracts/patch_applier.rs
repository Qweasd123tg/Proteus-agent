use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

use crate::domain::{Patch, PatchResult};

#[async_trait]
pub trait PatchApplier: Send + Sync {
    /// Apply relative to the invocation directory within the bound workspace.
    /// Relative invocation directories resolve from that workspace, not process cwd.
    async fn apply(&self, patch: Patch, cwd: &Path) -> Result<PatchResult>;
}
