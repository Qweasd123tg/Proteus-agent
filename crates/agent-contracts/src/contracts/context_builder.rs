use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{MemoryStore, SearchBackend},
    domain::{AgentTask, ContextBundle},
};

#[derive(Clone)]
pub struct ContextBuildInput {
    pub task: AgentTask,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
}

impl ContextBuildInput {
    pub fn new(
        task: AgentTask,
        search: Arc<dyn SearchBackend>,
        memory: Arc<dyn MemoryStore>,
    ) -> Self {
        Self {
            task,
            search,
            memory,
        }
    }
}

#[async_trait]
pub trait ContextBuilder: Send + Sync {
    async fn build(&self, input: ContextBuildInput) -> Result<ContextBundle>;
}
