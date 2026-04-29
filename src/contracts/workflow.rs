use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{
        ApprovalPolicy, ApprovalTransport, ContextBuilder, EventEmitter, MemoryStore, ModelClient,
        PatchApplier, SearchBackend, ToolRegistry,
    },
    domain::{AgentOutput, AgentTask, Event, EventContext, ModelRef, SessionId, ThreadId, TurnId},
    model_standard::CanonicalMessage,
};

#[derive(Clone)]
#[non_exhaustive]
pub struct RuntimeContext {
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub model_ref: ModelRef,
    pub model_timeout_ms: u64,
    pub context_timeout_ms: u64,
    pub events: Arc<EventEmitter>,
    pub model: Arc<dyn ModelClient>,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
    pub context: Arc<dyn ContextBuilder>,
    pub tools: ToolRegistry,
    pub policy: Arc<dyn ApprovalPolicy>,
    pub approval: Arc<dyn ApprovalTransport>,
    pub patch: Arc<dyn PatchApplier>,
}

impl RuntimeContext {
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
