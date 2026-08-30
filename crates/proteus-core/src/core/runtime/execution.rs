use std::sync::Arc;

use anyhow::Result;

use crate::{
    contracts::{
        CancellationToken, ExecutionPermissionGrants, ExecutionScope, NoopToolExecutionRecorder,
        ToolExecutionRecorder,
    },
    core::{
        BoundTools, ModeAwarePolicy, SessionConfigSnapshot, SessionToolExecutionRecorder,
        ToolExecutionBinding,
    },
    domain::{ModelRef, PermissionMode, ReasoningConfig, ToolCall, ToolResult},
};

use super::{AgentRuntime, RuntimeSnapshot};

/// Coherent effective runtime state captured once for one admitted execution.
///
/// This stays private to `AgentRuntime`: callers receive only typed operation
/// inputs/results, never the registry or a broad ambient service context.
#[derive(Clone)]
pub(super) struct ExecutionAdmissionSnapshot {
    pub(super) runtime: RuntimeSnapshot,
    pub(super) permission_mode: PermissionMode,
    pub(super) model_ref: ModelRef,
    pub(super) reasoning: ReasoningConfig,
    pub(super) config_snapshot: Option<SessionConfigSnapshot>,
}

/// One top-level admission: exactly one immutable snapshot and one scope.
pub(super) struct ExecutionAdmission {
    pub(super) scope: ExecutionScope,
    pub(super) snapshot: ExecutionAdmissionSnapshot,
}

impl AgentRuntime {
    /// Runs one configured tool as a top-level logical execution without
    /// creating chat/session lifecycle state.
    ///
    /// Registry selection, policy mode, approval, grants, cancellation and
    /// recording are bound before the canonical call reaches the tool. The
    /// caller never receives the underlying registry or service context.
    pub async fn execute_tool(
        &self,
        call: ToolCall,
        cancellation: CancellationToken,
    ) -> Result<ToolResult> {
        let admission = self.admit_execution(cancellation).await;
        let tools = self.bind_detached_tools(&admission);
        tools.execute(self.services.cwd.clone(), call).await
    }

    pub(super) async fn admit_execution(
        &self,
        cancellation: CancellationToken,
    ) -> ExecutionAdmission {
        ExecutionAdmission {
            scope: ExecutionScope::fresh(cancellation),
            snapshot: self.capture_execution_snapshot().await,
        }
    }

    pub(super) async fn capture_execution_snapshot(&self) -> ExecutionAdmissionSnapshot {
        let state = self.services.execution_state.read().await;
        let mut config_snapshot = state.runtime.config_snapshot.clone();
        if let Some(config) = &mut config_snapshot {
            config.model = state.model_ref.clone();
            config.reasoning = state.reasoning.clone();
            config.permission_mode_default = state.permission_mode;
        }
        ExecutionAdmissionSnapshot {
            runtime: state.runtime.clone(),
            permission_mode: state.permission_mode,
            model_ref: state.model_ref.clone(),
            reasoning: state.reasoning.clone(),
            config_snapshot,
        }
    }

    fn bind_detached_tools(&self, admission: &ExecutionAdmission) -> BoundTools {
        let recorder: Arc<dyn ToolExecutionRecorder> = match &self.session.session_store {
            Some(store) => Arc::new(SessionToolExecutionRecorder::new(store.clone())),
            None => Arc::new(NoopToolExecutionRecorder),
        };
        let binding =
            ToolExecutionBinding::detached(admission.scope.clone()).with_recorder(recorder);
        let registry = &admission.snapshot.runtime.registry;
        BoundTools::new(
            registry.tools.clone(),
            Arc::new(ModeAwarePolicy::new(
                admission.snapshot.permission_mode,
                registry.policy.clone(),
            )),
            self.services.approval.clone(),
            Arc::<ExecutionPermissionGrants>::default(),
            binding,
        )
    }
}
