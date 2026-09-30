use super::{AgentRuntime, ExecutionAdmissionSnapshot};
use crate::{
    contracts::{
        ExecutionAttribution, ExecutionRecorder, ExecutionScope, HookEvent, HookInput,
        HookTurnStatus, NoopExecutionRecorder,
    },
    core::{SessionExecutionRecorder, TurnSettled, TurnSettlementStatus},
};
use std::sync::Arc;

impl AgentRuntime {
    pub(super) async fn notify_turn_started(
        &self,
        context: &crate::contracts::AgentWorkflowContext,
        task: &crate::domain::AgentTask,
        history: &[crate::model_standard::CanonicalMessage],
    ) {
        if let Err(error) = context
            .execution
            .hooks
            .apply(HookInput {
                event: HookEvent::TurnStarted {
                    task: task.clone(),
                    history: history.to_vec(),
                },
                attribution: ExecutionAttribution::for_turn(
                    context.execution.scope.execution_id,
                    self.session.session_id,
                    self.session.thread_id,
                    context.turn_id,
                ),
                cwd: task.cwd.clone(),
            })
            .await
        {
            eprintln!("warning: turn_started hook recording failed: {error:#}");
        }
    }

    pub(super) async fn notify_turn_settled(
        &self,
        snapshot: &ExecutionAdmissionSnapshot,
        scope: ExecutionScope,
        attribution: ExecutionAttribution,
        settlement: &TurnSettled,
    ) {
        let recorder: Arc<dyn ExecutionRecorder> = match &self.session.session_store {
            Some(store) => Arc::new(SessionExecutionRecorder::for_turn(
                store.clone(),
                scope.execution_id,
                self.session.thread_id,
                attribution.agent.expect("root turn attribution").turn_id,
            )),
            None => Arc::new(NoopExecutionRecorder),
        };
        let status = match settlement.status {
            TurnSettlementStatus::Success => HookTurnStatus::Success,
            TurnSettlementStatus::Error => HookTurnStatus::Error,
            TurnSettlementStatus::Canceled => HookTurnStatus::Canceled,
            TurnSettlementStatus::Timeout => HookTurnStatus::Timeout,
        };
        let hooks = snapshot.runtime.registry.bind_hooks(scope, recorder);
        if let Err(error) = hooks
            .apply(HookInput {
                event: HookEvent::TurnSettled {
                    status,
                    output: settlement.output.clone(),
                    error: settlement.error.clone(),
                },
                attribution,
                cwd: self.services.cwd.clone(),
            })
            .await
        {
            eprintln!("warning: turn_settled hook recording failed: {error:#}");
        }
    }
}
