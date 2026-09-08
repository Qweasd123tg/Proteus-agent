use std::sync::Mutex;

use super::{
    terminal::{TerminalModel, terminal_journal},
    *,
};
use crate::{
    contracts::ModelCallOrigin,
    model_standard::{MessagePhase, ModelFailure, ModelFailureKind},
};

struct FailureRoutingWorkflow {
    observed: Arc<Mutex<Option<ModelFailure>>>,
}

#[async_trait]
impl Workflow for FailureRoutingWorkflow {
    async fn run(
        &self,
        _task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: AgentWorkflowContext,
    ) -> anyhow::Result<WorkflowOutput> {
        let request = CanonicalModelRequest::new(ctx.model_ref.clone(), history)
            .with_tools(vec![probe_tool_spec()]);
        let error = ctx
            .execution
            .model
            .complete(request)
            .await
            .expect_err("model must fail");
        let failure = ModelFailure::from_error(&error);
        *self.observed.lock().unwrap() = Some(failure.clone());
        // The model returns the same message for every cause. Only the typed
        // cause may select this workflow's terminal branch.
        match failure.kind {
            ModelFailureKind::ContextWindowExceeded => anyhow::bail!("branch:context-window"),
            ModelFailureKind::Interrupted => anyhow::bail!("branch:interrupted"),
            ModelFailureKind::SessionBudgetExceeded => anyhow::bail!("branch:budget"),
            ModelFailureKind::Other => anyhow::bail!("branch:other"),
            _ => anyhow::bail!("branch:unknown"),
        }
    }
}

#[tokio::test]
async fn recorded_failure_kind_selects_the_same_workflow_branch() {
    for (kind, terminal) in [
        (
            ModelFailureKind::ContextWindowExceeded,
            "branch:context-window",
        ),
        (ModelFailureKind::Interrupted, "branch:interrupted"),
        (ModelFailureKind::SessionBudgetExceeded, "branch:budget"),
        (ModelFailureKind::Other, "branch:other"),
    ] {
        let completed = CanonicalMessage::text(MessageRole::Assistant, "accepted progress")
            .with_phase(MessagePhase::Commentary);
        let failure = ModelFailure::new(kind, "same provider error text")
            .with_completed_messages(vec![completed]);
        let journal = terminal_journal(
            TurnSettlementStatus::Error,
            ModelCallOrigin::Direct,
            TerminalModel::Failure(failure.clone()),
            terminal,
        )
        .await;
        let observed = Arc::new(Mutex::new(None));
        let mut catalog = ModuleCatalog::new();
        catalog.register_test_workflow(
            WORKFLOW_ID,
            Arc::new(FailureRoutingWorkflow {
                observed: observed.clone(),
            }),
        );
        catalog.register_test_policy(POLICY_ID, Arc::new(ReplayAllowAll));

        let report = replay_workflow(
            journal.store.session_dir(),
            &AppConfig::default(),
            &catalog,
            WorkflowReplayOptions::default(),
        )
        .await
        .expect("typed failure workflow replay");

        assert!(
            report.comparison.matched,
            "{kind:?}: {:?}",
            report.comparison.issues
        );
        assert_eq!(report.recorded.status, TurnSettlementStatus::Error);
        assert_eq!(report.replay.status, TurnSettlementStatus::Error);
        assert_eq!(report.comparison.error_equal, Some(true));
        assert_eq!(report.comparison.history_equal, Some(true));
        assert_eq!(report.model_exchanges.recorded, 1);
        assert_eq!(report.model_exchanges.replayed, 1);
        assert_eq!(*observed.lock().unwrap(), Some(failure));
        assert!(report.source_journal_unchanged);
    }
}
