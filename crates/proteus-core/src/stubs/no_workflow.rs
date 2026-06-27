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
        task: AgentTask,
        mut history: Vec<CanonicalMessage>,
        _ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        let output = AgentOutput::text(
            "workflow is disabled; select a workflow plugin such as coding.plan_execute_review",
        );
        let new_messages_start = history.len();
        history.push(CanonicalMessage::text(MessageRole::User, task.text));
        history.push(CanonicalMessage::text(
            MessageRole::Assistant,
            output.text.clone(),
        ));
        Ok(WorkflowOutput::new(output, history).with_new_messages_start(new_messages_start))
    }
}
