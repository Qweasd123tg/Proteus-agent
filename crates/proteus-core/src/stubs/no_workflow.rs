use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{RuntimeContext, Workflow, WorkflowOutput},
    domain::{AgentOutput, AgentTask},
    model_standard::CanonicalMessage,
};

#[derive(Debug, Default)]
pub struct NoWorkflow;

#[async_trait]
impl Workflow for NoWorkflow {
    async fn run(
        &self,
        _task: AgentTask,
        history: Vec<CanonicalMessage>,
        _ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        Ok(WorkflowOutput::new(
            AgentOutput::text(
                "workflow is disabled; select a workflow plugin such as coding.plan_execute_review",
            ),
            history,
        ))
    }
}
