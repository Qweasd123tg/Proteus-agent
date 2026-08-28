use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{AgentWorkflowContext, Workflow, WorkflowOutput},
    domain::{AgentOutput, AgentTask},
    model_standard::{CanonicalMessage, MessageRole},
};

#[derive(Debug, Default)]
pub struct NoWorkflow;

#[async_trait]
impl Workflow for NoWorkflow {
    async fn run(
        &self,
        _task: AgentTask,
        _history: Vec<CanonicalMessage>,
        _ctx: AgentWorkflowContext,
    ) -> Result<WorkflowOutput> {
        let output = AgentOutput::text(
            "no workflow module is selected; set modules.workflow and add its process descriptor",
        );
        let assistant_message = CanonicalMessage::text(MessageRole::Assistant, output.text.clone());
        Ok(WorkflowOutput::new(output, vec![assistant_message]))
    }
}
