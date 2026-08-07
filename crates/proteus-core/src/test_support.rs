use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{
        ApprovalPolicy, PolicyContext, PolicyVisibilityContext, RuntimeContext, Workflow,
        WorkflowOutput,
    },
    core::{ModuleCatalog, ToolOrchestrator},
    domain::{AgentOutput, AgentTask, PolicyDecision, ToolCall},
    model_standard::{CanonicalMessage, CanonicalModelRequest, ContentPart, MessageRole},
    stubs::EmptyContextBuilder,
};

pub(crate) fn module_catalog() -> ModuleCatalog {
    let mut catalog = ModuleCatalog::new();
    catalog.register_test_context("simple", Arc::new(EmptyContextBuilder));
    catalog.register_test_workflow("coding.single_loop", Arc::new(TestToolLoopWorkflow));
    catalog.register_test_workflow("coding.plan_execute_review", Arc::new(TestToolLoopWorkflow));
    catalog.register_test_policy("ask_write", Arc::new(TestAskWritePolicy));
    catalog
}

pub(crate) fn select_test_modules(config: &mut crate::core::AppConfig, workflow: &str) {
    config.modules.workflow = Some(workflow.to_owned());
    config.modules.context = Some("simple".to_owned());
    config.modules.policy = Some("ask_write".to_owned());
}

struct TestAskWritePolicy;

impl ApprovalPolicy for TestAskWritePolicy {
    fn evaluate(&self, call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
        if call.name == "apply_patch" {
            PolicyDecision::Ask {
                reason: "test write approval".to_owned(),
            }
        } else {
            PolicyDecision::Allow
        }
    }

    fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

struct TestToolLoopWorkflow;

#[async_trait]
impl Workflow for TestToolLoopWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: RuntimeContext,
    ) -> Result<WorkflowOutput> {
        let mut messages = history;
        let mut new_messages = Vec::new();
        let orchestrator = ToolOrchestrator::default();

        for _ in 0..4 {
            let request = CanonicalModelRequest::new(ctx.model_ref.clone(), messages.clone())
                .with_instructions(ctx.instructions.clone())
                .with_tools(ctx.tools.specs())
                .with_reasoning(ctx.reasoning.clone());
            let response = ctx.model.complete(request).await?;
            messages.push(response.message.clone());
            new_messages.push(response.message.clone());

            if response.tool_calls.is_empty() {
                return Ok(WorkflowOutput::new(
                    AgentOutput::text(message_text(&response.message)),
                    new_messages,
                ));
            }

            for call in response.tool_calls {
                let result = orchestrator.execute(&ctx, &task, call.clone()).await?;
                let message = CanonicalMessage::new(
                    MessageRole::Tool,
                    vec![ContentPart::ToolResult {
                        result: result.clone(),
                    }],
                )
                .with_tool_call_id(call.id);
                messages.push(message.clone());
                new_messages.push(message);
            }
        }

        anyhow::bail!("test workflow exceeded tool round limit")
    }
}

fn message_text(message: &CanonicalMessage) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match &part.payload {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
