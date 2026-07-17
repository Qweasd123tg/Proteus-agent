use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{
        ApprovalPolicy, ApprovalTransport, CancellationToken, ContextBuilder, EventEmitter,
        HistoryCompactor, MemoryStore, Model, PatchApplier, SearchBackend, SubagentRunner,
        ToolExposure, ToolRegistry, TurnPermissionGrants, UserInputTransport,
    },
    domain::{
        AgentOutput, AgentTask, Event, EventContext, HistoryCompactionReport, ModelRef,
        ReasoningConfig, SessionId, ThreadId, TurnId,
    },
    model_standard::{CanonicalMessage, InstructionBlock},
};

#[derive(Clone)]
#[non_exhaustive]
pub struct RuntimeContext {
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub model_ref: ModelRef,
    pub instructions: Vec<InstructionBlock>,
    pub reasoning: ReasoningConfig,
    pub model_timeout_ms: u64,
    pub context_timeout_ms: u64,
    pub cancellation: CancellationToken,
    pub events: Arc<EventEmitter>,
    pub model: Arc<dyn Model>,
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
    pub subagent: Arc<dyn SubagentRunner>,
    /// Turn-scoped permission grants: контекст создаётся на каждый ход
    /// заново, поэтому гранты не переживают ход (см. `TurnPermissionGrants`).
    pub turn_grants: Arc<TurnPermissionGrants>,
    /// Человекочитаемая метка исполняющего thread-а для attribution
    /// (approvals, клиентский UX). `None` — основной цикл turn-а; субагентный
    /// runner ставит имя роли.
    pub thread_label: Option<String>,
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
        model: Arc<dyn Model>,
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
        subagent: Arc<dyn SubagentRunner>,
    ) -> Self {
        Self {
            session_id,
            thread_id,
            turn_id,
            model_ref,
            instructions: Vec::new(),
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
            subagent,
            turn_grants: Arc::default(),
            thread_label: None,
        }
    }

    pub fn with_thread_label(mut self, label: impl Into<String>) -> Self {
        self.thread_label = Some(label.into());
        self
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub fn with_instructions(mut self, instructions: Vec<InstructionBlock>) -> Self {
        self.instructions = instructions;
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
    /// Persistent assistant/tool messages produced after the current user
    /// message from the input history.
    pub new_messages: Vec<CanonicalMessage>,
    /// Compacted persistent history snapshot that preserves the exact current
    /// user message from the input history.
    ///
    /// `None` keeps the runtime history append-only. `Some` asks the runtime
    /// to atomically replace the existing history with this snapshot before it
    /// appends `new_messages`.
    pub history_replacement: Option<Vec<CanonicalMessage>>,
    pub compactions: Vec<HistoryCompactionReport>,
}

impl WorkflowOutput {
    pub fn new(output: AgentOutput, new_messages: Vec<CanonicalMessage>) -> Self {
        Self {
            output,
            new_messages,
            history_replacement: None,
            compactions: Vec::new(),
        }
    }

    pub fn with_history_replacement(mut self, messages: Vec<CanonicalMessage>) -> Self {
        self.history_replacement = Some(messages);
        self
    }

    pub fn with_compactions(mut self, compactions: Vec<HistoryCompactionReport>) -> Self {
        self.compactions = compactions;
        self
    }
}
