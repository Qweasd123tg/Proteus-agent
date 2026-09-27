use std::sync::Arc;

use crate::{
    contracts::{
        AgentWorkflowContext, ExecutionRecorder, ExecutionScope, NoopExecutionRecorder,
        NoopToolExecutionRecorder, ToolExecutionRecorder,
    },
    core::{ModelExecutionBinding, SessionExecutionRecorder, SessionToolExecutionRecorder},
    domain::TurnId,
};

use super::{AgentRuntime, ExecutionAdmissionSnapshot};

impl AgentRuntime {
    /// Binds one admitted Turn to the generic execution mechanisms selected by
    /// its immutable runtime snapshot, then adds the agent/chat wrapper.
    ///
    /// This is intentionally an agent-layer adapter. Generic execution types do
    /// not depend on `TurnId`, while the normal interactive path preserves its
    /// journal and presentation attribution.
    pub(super) async fn bind_agent_workflow_context(
        &self,
        scope: ExecutionScope,
        snapshot: &ExecutionAdmissionSnapshot,
        turn_id: TurnId,
    ) -> anyhow::Result<AgentWorkflowContext> {
        if let Some(store) = &self.session.session_store {
            let records = store.load_records()?;
            *self.session.model_context.lock().await =
                crate::core::model_context::ModelContextState::from_records(
                    &records,
                    self.session.thread_id,
                    Some(turn_id),
                );
        }
        let model_context = self.session.model_context.lock().await.snapshot();
        let execution_id = scope.execution_id;
        let execution_recorder: Arc<dyn ExecutionRecorder> = match &self.session.session_store {
            Some(store) => Arc::new(SessionExecutionRecorder::for_turn(
                store.clone(),
                execution_id,
                self.session.thread_id,
                turn_id,
            )),
            None => Arc::new(NoopExecutionRecorder),
        };
        let tool_recorder: Arc<dyn ToolExecutionRecorder> = match &self.session.session_store {
            Some(store) => Arc::new(SessionToolExecutionRecorder::new(store.clone())),
            None => Arc::new(NoopToolExecutionRecorder),
        };
        let execution_recorder = Arc::new(
            crate::core::model_context::recorder::ContextExecutionRecorder {
                inner: execution_recorder,
                context: self.session.model_context.clone(),
            },
        );
        let model_binding = ModelExecutionBinding::for_turn(
            scope,
            self.services.events.clone(),
            self.session.session_id,
            self.session.thread_id,
            turn_id,
            execution_recorder,
        );
        let execution = snapshot.runtime.registry.execution_context(
            model_binding,
            self.services.approval.clone(),
            snapshot.permission_mode,
        );

        let mut context = snapshot
            .runtime
            .registry
            .agent_workflow_context(
                execution,
                self.session.session_id,
                self.session.thread_id,
                turn_id,
                snapshot.model_ref.clone(),
                snapshot.reasoning.clone(),
                self.services.events.clone(),
                self.services.user_input.clone(),
            )
            .with_tool_recorder(tool_recorder);
        context.model_context = model_context;
        Ok(context)
    }
}
