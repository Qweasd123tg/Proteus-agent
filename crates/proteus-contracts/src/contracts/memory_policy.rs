use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::MemoryStore,
    domain::{AgentOutput, AgentTask},
    model_standard::CanonicalMessage,
};

pub struct MemoryPolicyInput<'a> {
    pub task: &'a AgentTask,
    pub output: &'a AgentOutput,
    pub new_messages: &'a [CanonicalMessage],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryPolicyOutput {
    pub written_kinds: Vec<String>,
}

#[async_trait]
pub trait MemoryPolicy: Send + Sync {
    async fn after_turn(
        &self,
        input: MemoryPolicyInput<'_>,
        memory: &dyn MemoryStore,
    ) -> Result<MemoryPolicyOutput>;
}
