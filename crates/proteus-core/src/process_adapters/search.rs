use std::{path::Path, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{
        PROCESS_SEARCH_CONTRACT_VERSION, PROCESS_SEARCH_METHOD, ProcessSearchResponse,
        SearchBackend, SearchQuery,
    },
    domain::ContextChunk,
};

use super::{ProcessAdapterConfig, ProcessModuleClient};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// `SearchBackend` implemented by one persistent process module.
pub struct ProcessSearchBackend {
    client: Arc<ProcessModuleClient>,
}

impl ProcessSearchBackend {
    pub fn new(config: ProcessAdapterConfig, workspace: &Path) -> Result<Self> {
        Ok(Self {
            client: Arc::new(ProcessModuleClient::connect(
                "search",
                PROCESS_SEARCH_CONTRACT_VERSION,
                config,
                workspace,
                DEFAULT_TIMEOUT_MS,
            )?),
        })
    }
}

#[async_trait]
impl SearchBackend for ProcessSearchBackend {
    async fn search(&self, query: SearchQuery) -> Result<Vec<ContextChunk>> {
        let client = Arc::clone(&self.client);
        tokio::task::spawn_blocking(move || {
            let response: ProcessSearchResponse = client.invoke(PROCESS_SEARCH_METHOD, &query)?;
            Ok(response.chunks)
        })
        .await
        .map_err(|error| anyhow::anyhow!("process search join error: {error}"))?
    }
}
