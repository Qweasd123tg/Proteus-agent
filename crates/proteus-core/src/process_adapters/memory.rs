use std::{path::Path, sync::Arc};

use crate::{
    contracts::{
        MemoryInvocationContext, MemoryStore, PROCESS_MEMORY_CONTRACT_VERSION,
        PROCESS_MEMORY_RECALL_METHOD, PROCESS_MEMORY_REMEMBER_METHOD, ProcessMemoryRecallInput,
        ProcessMemoryRecallResponse, ProcessMemoryRememberInput, ProcessMemoryRememberResponse,
    },
    domain::{MemoryItem, MemoryQuery},
};
use anyhow::Result;
use async_trait::async_trait;
use proteus_module_protocol::v3::NoAsyncHostRequests;

use super::{ProcessExportClient, ProcessExportConfig};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct ProcessMemoryStore {
    client: Arc<ProcessExportClient>,
}

impl ProcessMemoryStore {
    pub fn new(config: ProcessExportConfig, workspace: &Path) -> Result<Self> {
        Ok(Self {
            client: Arc::new(ProcessExportClient::connect(
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
    async fn remember(&self, item: MemoryItem, ctx: MemoryInvocationContext) -> Result<()> {
        let cancellation = ctx.cancellation;
        let response: ProcessMemoryRememberResponse = self
            .client
            .invoke_with_dispatcher_and_cancel_check(
                PROCESS_MEMORY_REMEMBER_METHOD,
                &ProcessMemoryRememberInput {
                    item,
                    attribution: ctx.attribution,
                },
                Arc::new(NoAsyncHostRequests),
                move || cancellation.is_cancelled(),
            )
            .await?;
        Ok(response.result)
    }

    async fn recall(
        &self,
        query: MemoryQuery,
        ctx: MemoryInvocationContext,
    ) -> Result<Vec<MemoryItem>> {
        let cancellation = ctx.cancellation;
        let response: ProcessMemoryRecallResponse = self
            .client
            .invoke_with_dispatcher_and_cancel_check(
                PROCESS_MEMORY_RECALL_METHOD,
                &ProcessMemoryRecallInput {
                    query,
                    attribution: ctx.attribution,
                },
                Arc::new(NoAsyncHostRequests),
                move || cancellation.is_cancelled(),
            )
            .await?;
        Ok(response.result)
    }
}
