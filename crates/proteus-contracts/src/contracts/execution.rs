use std::sync::Arc;

use crate::{
    contracts::{
        ApprovalPolicy, ApprovalTransport, CancellationToken, ExecutionRecorder, MemoryStore,
        Model, NoopExecutionRecorder, PatchApplier, SearchBackend, ToolRegistry,
    },
    domain::{ExecutionId, new_execution_id},
};

/// Generic identity and cancellation boundary for one logical workload.
///
/// The scope deliberately owns no runtime services, chat identity, history or
/// process-broker lineage.
#[derive(Debug, Clone)]
pub struct ExecutionScope {
    pub execution_id: ExecutionId,
    pub cancellation: CancellationToken,
}

impl ExecutionScope {
    pub fn new(execution_id: ExecutionId, cancellation: CancellationToken) -> Self {
        Self {
            execution_id,
            cancellation,
        }
    }

    pub fn fresh(cancellation: CancellationToken) -> Self {
        Self::new(new_execution_id(), cancellation)
    }

    /// Creates a targeted cancellation view of the same logical execution.
    /// This is not a child execution and therefore preserves `ExecutionId`.
    pub fn child_cancellation_scope(&self) -> Self {
        Self::new(self.execution_id, self.cancellation.child_token())
    }
}

/// Generic runtime dependencies bound to one coherent execution snapshot.
///
/// This is a migration boundary, not a promise that a broad context object is
/// the final capability API. It deliberately contains no conversational
/// identity, history, agent task or presentation state.
#[derive(Clone)]
#[non_exhaustive]
pub struct ExecutionContext {
    pub scope: ExecutionScope,
    pub model_timeout_ms: u64,
    pub model: Arc<dyn Model>,
    pub search: Arc<dyn SearchBackend>,
    pub memory: Arc<dyn MemoryStore>,
    pub tools: ToolRegistry,
    pub policy: Arc<dyn ApprovalPolicy>,
    pub approval: Arc<dyn ApprovalTransport>,
    pub patch: Arc<dyn PatchApplier>,
    pub execution_recorder: Arc<dyn ExecutionRecorder>,
}

impl ExecutionContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope: ExecutionScope,
        model_timeout_ms: u64,
        model: Arc<dyn Model>,
        search: Arc<dyn SearchBackend>,
        memory: Arc<dyn MemoryStore>,
        tools: ToolRegistry,
        policy: Arc<dyn ApprovalPolicy>,
        approval: Arc<dyn ApprovalTransport>,
        patch: Arc<dyn PatchApplier>,
    ) -> Self {
        Self {
            scope,
            model_timeout_ms,
            model,
            search,
            memory,
            tools,
            policy,
            approval,
            patch,
            execution_recorder: Arc::new(NoopExecutionRecorder),
        }
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.scope = ExecutionScope::new(self.scope.execution_id, cancellation);
        self
    }

    pub fn with_execution_recorder(mut self, recorder: Arc<dyn ExecutionRecorder>) -> Self {
        self.execution_recorder = recorder;
        self
    }

    pub fn is_cancelled(&self) -> bool {
        self.scope.cancellation.is_cancelled()
    }
}
