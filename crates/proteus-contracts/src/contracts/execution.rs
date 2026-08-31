use std::sync::Arc;

use crate::{
    contracts::{
        ApprovalPolicy, ApprovalTransport, CancellationToken, ExecutionPermissionGrants,
        MemoryStore, Model, SearchBackend, ToolRegistry,
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
    /// Mutable authority issued during this execution binding. It is separate
    /// from the identity/cancellation-only `ExecutionScope`.
    pub permission_grants: Arc<ExecutionPermissionGrants>,
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
            permission_grants: Arc::default(),
        }
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.scope = ExecutionScope::new(self.scope.execution_id, cancellation);
        self
    }

    /// Attenuates authority for a narrower binding without creating a new
    /// logical `ExecutionId` (for example, an isolated child agent).
    pub fn with_fresh_permission_grants(mut self) -> Self {
        self.permission_grants = Arc::default();
        self
    }

    pub fn is_cancelled(&self) -> bool {
        self.scope.cancellation.is_cancelled()
    }
}
