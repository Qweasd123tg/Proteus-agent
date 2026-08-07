use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{RuntimeContext, Workflow, WorkflowOutput},
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
        _ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        let output = AgentOutput::text(
            "workflow is disabled; select a Workflow implementation such as modules.workflow = process",
        );
        let assistant_message = CanonicalMessage::text(MessageRole::Assistant, output.text.clone());
        Ok(WorkflowOutput::new(output, vec![assistant_message]))
    }
}
