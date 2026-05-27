use anyhow::Result;
use async_trait::async_trait;

use crate::contracts::{CompactionInput, CompactionOutput, HistoryCompactor};

#[derive(Debug, Default, Clone)]
pub struct NoCompactor;

#[async_trait]
impl HistoryCompactor for NoCompactor {
    async fn compact(&self, input: CompactionInput) -> Result<CompactionOutput> {
        Ok(CompactionOutput::unchanged(input.messages))
    }
}
