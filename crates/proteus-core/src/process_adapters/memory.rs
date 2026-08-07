use std::{path::Path, sync::Arc};

use crate::{
    contracts::{
        MemoryStore, PROCESS_MEMORY_CONTRACT_VERSION, PROCESS_MEMORY_RECALL_METHOD,
        PROCESS_MEMORY_REMEMBER_METHOD, ProcessMemoryRecallInput, ProcessMemoryRecallResponse,
        ProcessMemoryRememberInput, ProcessMemoryRememberResponse,
    },
    domain::{MemoryItem, MemoryQuery},
};
use anyhow::Result;
use async_trait::async_trait;

use super::{ProcessAdapterConfig, ProcessModuleClient};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct ProcessMemoryStore {
    client: Arc<ProcessModuleClient>,
}

impl ProcessMemoryStore {
    pub fn new(config: ProcessAdapterConfig, workspace: &Path) -> Result<Self> {
        Ok(Self {
            client: Arc::new(ProcessModuleClient::connect(
                "memory",
                PROCESS_MEMORY_CONTRACT_VERSION,
                config,
                workspace,
                DEFAULT_TIMEOUT_MS,
            )?),
        })
    }
}

#[async_trait]
impl MemoryStore for ProcessMemoryStore {
    async fn remember(&self, item: MemoryItem) -> Result<()> {
        let client = Arc::clone(&self.client);
        tokio::task::spawn_blocking(move || {
            let response: ProcessMemoryRememberResponse = client.invoke(
                PROCESS_MEMORY_REMEMBER_METHOD,
                &ProcessMemoryRememberInput { item },
            )?;
            Ok(response.result)
        })
        .await
        .map_err(|error| anyhow::anyhow!("process memory join error: {error}"))?
    }

    async fn recall(&self, query: MemoryQuery) -> Result<Vec<MemoryItem>> {
        let client = Arc::clone(&self.client);
        tokio::task::spawn_blocking(move || {
            let response: ProcessMemoryRecallResponse = client.invoke(
                PROCESS_MEMORY_RECALL_METHOD,
                &ProcessMemoryRecallInput { query },
            )?;
            Ok(response.result)
        })
        .await
        .map_err(|error| anyhow::anyhow!("process memory join error: {error}"))?
    }
}
