use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    contracts::{
        ApprovalPolicy, ApprovalTransport, ContextBuilder, EventSink, MemoryStore, ModelClient,
        PatchApplier, SearchBackend, ToolRegistry,
    },
    domain::{AgentOutput, AgentTask, ModelRef, PermissionMode, SessionId},
    model_standard::CanonicalMessage,
};

#[derive(Clone)]
pub struct RuntimeContext {
    pub session_id: SessionId,
    pub model_ref: ModelRef,
    pub event_sink: Arc<dyn EventSink>,
    pub model: Arc<dyn ModelClient>,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
    pub context: Arc<dyn ContextBuilder>,
    pub tools: ToolRegistry,
    pub policy: Arc<dyn ApprovalPolicy>,
    pub approval: Arc<dyn ApprovalTransport>,
    pub permission_mode: PermissionMode,
    pub patch: Arc<dyn PatchApplier>,
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
pub struct WorkflowOutput {
    pub output: AgentOutput,
    pub messages: Vec<CanonicalMessage>,
}
