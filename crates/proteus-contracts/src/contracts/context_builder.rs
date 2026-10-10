use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{ExecutionAttribution, ExecutionScope},
    domain::{AgentTask, ContextBundle},
};

#[derive(Clone)]
pub struct ContextBuildInput {
    pub task: AgentTask,
    pub scope: ExecutionScope,
    pub attribution: ExecutionAttribution,
}

impl ContextBuildInput {
    pub fn new(task: AgentTask, scope: ExecutionScope, attribution: ExecutionAttribution) -> Self {
        Self {
            task,
            scope,
            attribution,
        }
    }
}

#[async_trait]
pub trait ContextBuilder: Send + Sync {
    async fn build(&self, input: ContextBuildInput) -> Result<ContextBundle>;
}
