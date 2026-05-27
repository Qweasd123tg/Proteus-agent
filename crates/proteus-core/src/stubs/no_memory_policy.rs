use anyhow::Result;
use async_trait::async_trait;

use crate::contracts::{MemoryPolicy, MemoryPolicyInput, MemoryPolicyOutput, MemoryStore};

#[derive(Debug)]
pub struct NoMemoryPolicy;

#[async_trait]
impl MemoryPolicy for NoMemoryPolicy {
    async fn after_turn(
        &self,
        _input: MemoryPolicyInput<'_>,
        _memory: &dyn MemoryStore,
    ) -> Result<MemoryPolicyOutput> {
        Ok(MemoryPolicyOutput::default())
    }
}
