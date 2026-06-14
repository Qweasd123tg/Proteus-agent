use anyhow::Result;
use async_trait::async_trait;

use std::sync::Arc;

use crate::contracts::{CompactionHost, CompactionInput, CompactionOutput, HistoryCompactor};

#[derive(Debug, Default, Clone)]
pub struct NoCompactor;

#[async_trait]
impl HistoryCompactor for NoCompactor {
    async fn compact(
        &self,
        input: CompactionInput,
        _host: Arc<dyn CompactionHost>,
    ) -> Result<CompactionOutput> {
        Ok(CompactionOutput::unchanged(input.messages))
    }
}
