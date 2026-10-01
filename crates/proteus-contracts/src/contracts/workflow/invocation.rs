use super::{AgentWorkflowContext, ProcessWorkflowRuntimeInfo, WorkflowConversationIdentity};
use crate::{
    contracts::{ContextBuilder, ExecutionContext, ToolExposure},
    domain::{ModelRef, ReasoningConfig},
    model_standard::InstructionBlock,
};
use anyhow::{Result, anyhow};
use std::sync::Arc;

/// The selected workflow receives the same execution capabilities in both
/// standalone and conversational invocations. The latter adds application state.
#[derive(Clone)]
pub enum WorkflowInvocationContext {
    Execution(WorkflowExecutionContext),
    Agent(AgentWorkflowContext),
}

#[derive(Clone)]
pub struct WorkflowExecutionContext {
    pub execution: ExecutionContext,
    pub context: Arc<dyn ContextBuilder>,
    pub tool_exposure: Arc<dyn ToolExposure>,
    pub model_ref: Option<ModelRef>,
    pub reasoning: ReasoningConfig,
    pub instructions: Vec<InstructionBlock>,
    pub context_timeout_ms: u64,
    pub intent: Option<String>,
    pub permission_mode: crate::domain::PermissionMode,
}

impl From<AgentWorkflowContext> for WorkflowInvocationContext {
    fn from(ctx: AgentWorkflowContext) -> Self {
        Self::Agent(ctx)
    }
}
impl WorkflowInvocationContext {
    pub fn execution(&self) -> &ExecutionContext {
        match self {
            Self::Execution(ctx) => &ctx.execution,
            Self::Agent(ctx) => &ctx.execution,
        }
    }
    pub fn agent(&self) -> Result<&AgentWorkflowContext> {
        match self {
            Self::Agent(ctx) => Ok(ctx),
            Self::Execution(_) => Err(anyhow!("this operation requires a conversation context")),
        }
    }
    pub fn into_agent(self) -> Result<AgentWorkflowContext> {
        match self {
            Self::Agent(ctx) => Ok(ctx),
            Self::Execution(_) => Err(anyhow!("this workflow requires a conversation context")),
        }
    }
    pub fn context(&self) -> Arc<dyn ContextBuilder> {
        match self {
            Self::Execution(ctx) => ctx.context.clone(),
            Self::Agent(ctx) => ctx.context.clone(),
        }
    }
    pub fn tool_exposure(&self) -> Arc<dyn ToolExposure> {
        match self {
            Self::Execution(ctx) => ctx.tool_exposure.clone(),
            Self::Agent(ctx) => ctx.tool_exposure.clone(),
        }
    }
    pub fn context_timeout_ms(&self) -> u64 {
        match self {
            Self::Execution(ctx) => ctx.context_timeout_ms,
            Self::Agent(ctx) => ctx.context_timeout_ms,
        }
    }
    pub fn runtime_info(&self, workflow_timeout_ms: u64) -> Result<ProcessWorkflowRuntimeInfo> {
        let (
            conversation,
            model_ref,
            reasoning,
            instructions,
            context_timeout_ms,
            intent,
            continuation,
            model_context,
            interrupted_turns,
            permission_mode,
        ) = match self {
            Self::Agent(ctx) => (
                Some(WorkflowConversationIdentity {
                    session_id: ctx.session_id,
                    thread_id: ctx.thread_id,
                    turn_id: ctx.turn_id,
                }),
                ctx.model_ref.clone(),
                ctx.reasoning.clone(),
                ctx.instructions.clone(),
                ctx.context_timeout_ms,
                ctx.intent.clone(),
                ctx.continuation.clone(),
                ctx.model_context.clone(),
                ctx.interrupted_turns.clone(),
                ctx.permission_mode,
            ),
            Self::Execution(ctx) => (
                None,
                ctx.model_ref.clone(),
                ctx.reasoning.clone(),
                ctx.instructions.clone(),
                ctx.context_timeout_ms,
                ctx.intent.clone(),
                None,
                vec![],
                vec![],
                ctx.permission_mode,
            ),
        };
        let execution = self.execution();
        let max_input_tokens = match &model_ref {
            Some(model) => {
                execution
                    .require_model()?
                    .capabilities(model)?
                    .max_input_tokens
            }
            None => None,
        };
        Ok(ProcessWorkflowRuntimeInfo {
            execution_id: execution.scope.execution_id,
            conversation,
            model_ref,
            model_context,
            interrupted_turns,
            instructions,
            intent,
            continuation,
            permission_mode,
            reasoning,
            max_input_tokens,
            model_timeout_ms: execution.model_timeout_ms,
            context_timeout_ms,
            workflow_timeout_ms,
        })
    }
}
