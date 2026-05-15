use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{
        ApprovalPolicy, ApprovalTransport, CancellationToken, ContextBuilder, EventEmitter,
        HistoryCompactor, MemoryStore, ModelClient, PatchApplier, SearchBackend, ToolExposure,
        ToolRegistry, UserInputTransport,
    },
    domain::{
        AgentOutput, AgentTask, Event, EventContext, ModelRef, ReasoningConfig, SessionId,
        ThreadId, TurnId,
    },
    model_standard::CanonicalMessage,
};

#[derive(Clone)]
#[non_exhaustive]
pub struct RuntimeContext {
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub model_ref: ModelRef,
    pub reasoning: ReasoningConfig,
    pub model_timeout_ms: u64,
    pub context_timeout_ms: u64,
    pub cancellation: CancellationToken,
    pub events: Arc<EventEmitter>,
    pub model: Arc<dyn ModelClient>,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
    pub context: Arc<dyn ContextBuilder>,
    pub tools: ToolRegistry,
    pub policy: Arc<dyn ApprovalPolicy>,
    pub approval: Arc<dyn ApprovalTransport>,
    pub user_input: Arc<dyn UserInputTransport>,
    pub patch: Arc<dyn PatchApplier>,
    pub compactor: Arc<dyn HistoryCompactor>,
    pub tool_exposure: Arc<dyn ToolExposure>,
}

impl RuntimeContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        model_ref: ModelRef,
        reasoning: ReasoningConfig,
        model_timeout_ms: u64,
        context_timeout_ms: u64,
        events: Arc<EventEmitter>,
        model: Arc<dyn ModelClient>,
        search: Arc<dyn SearchBackend>,
        memory: Arc<dyn MemoryStore>,
        context: Arc<dyn ContextBuilder>,
        tools: ToolRegistry,
        policy: Arc<dyn ApprovalPolicy>,
        approval: Arc<dyn ApprovalTransport>,
        user_input: Arc<dyn UserInputTransport>,
        patch: Arc<dyn PatchApplier>,
        compactor: Arc<dyn HistoryCompactor>,
        tool_exposure: Arc<dyn ToolExposure>,
    ) -> Self {
        Self {
            session_id,
            thread_id,
            turn_id,
            model_ref,
            reasoning,
            model_timeout_ms,
            context_timeout_ms,
            cancellation: CancellationToken::new(),
            events,
            model,
            search,
            memory,
            context,
            tools,
            policy,
            approval,
            user_input,
            patch,
            compactor,
            tool_exposure,
        }
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn event_context(&self) -> EventContext {
        EventContext {
            session_id: self.session_id,
            thread_id: self.thread_id,
            turn_id: Some(self.turn_id),
        }
    }

    pub async fn emit(&self, event: Event) -> Result<()> {
        self.events.emit(self.event_context(), event).await
    }
}

#[async_trait]
pub trait Workflow: Send + Sync {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: RuntimeContext,
    ) -> Result<WorkflowOutput>;
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorkflowOutput {
    pub output: AgentOutput,
    pub messages: Vec<CanonicalMessage>,
}

impl WorkflowOutput {
    pub fn new(output: AgentOutput, messages: Vec<CanonicalMessage>) -> Self {
        Self { output, messages }
    }
}
