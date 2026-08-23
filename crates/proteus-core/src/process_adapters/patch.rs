use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    contracts::{
        PROCESS_PATCH_APPLY_METHOD, PROCESS_PATCH_CONTRACT_VERSION, PatchApplier,
        ProcessPatchInput, ProcessPatchResponse,
    },
    domain::{Patch, PatchResult},
};
use anyhow::Result;
use async_trait::async_trait;

use super::{ProcessExportClient, ProcessExportConfig};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct ProcessPatchApplier {
    cwd: PathBuf,
    client: Arc<ProcessExportClient>,
}

impl ProcessPatchApplier {
    pub fn new(config: ProcessExportConfig, workspace: &Path) -> Result<Self> {
        Ok(Self {
            cwd: workspace.to_path_buf(),
            client: Arc::new(ProcessExportClient::connect(
                "patch",
                PROCESS_PATCH_CONTRACT_VERSION,
                config,
                workspace,
                DEFAULT_TIMEOUT_MS,
            )?),
        })
    }
}

#[async_trait]
impl PatchApplier for ProcessPatchApplier {
    async fn apply(&self, patch: Patch) -> Result<PatchResult> {
        let response: ProcessPatchResponse = self
            .client
            .invoke(
                PROCESS_PATCH_APPLY_METHOD,
                &ProcessPatchInput {
                    patch,
                    cwd: self.cwd.clone(),
                },
            )
            .await?;
        Ok(response.result)
    }
}
