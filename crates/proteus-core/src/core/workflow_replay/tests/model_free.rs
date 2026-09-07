use super::{
    terminal::{TerminalModel, terminal_journal},
    *,
};
use crate::{
    contracts::{
        CompactionInput, ContextBuildInput, MemoryInvocationContext, ToolExposureInput,
        ToolExposureRequest,
    },
    core::RuntimeCompactionHost,
};

const FAILURE: &str = "deterministic workflow failure";

#[derive(Clone, Copy)]
enum Probe {
    NoCalls,
    Model,
    Context,
    Tool,
    Exposure,
    Compaction,
}

#[async_trait]
impl Workflow for Probe {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: AgentWorkflowContext,
    ) -> anyhow::Result<WorkflowOutput> {
        // Deliberately swallow host errors: equal terminal errors alone must
        // never conceal a changed workflow's unrecorded host calls.
        match self {
            Self::NoCalls => {}
            Self::Model => {
                let request = CanonicalModelRequest::new(ctx.model_ref.clone(), history)
                    .with_tools(vec![probe_tool_spec()]);
                let _ = ctx.execution.model.complete(request).await;
            }
            Self::Context => {
                let input = ContextBuildInput::new(
                    task,
                    ctx.execution.search.clone(),
                    ctx.execution.memory.clone(),
                    MemoryInvocationContext::new(
                        ExecutionAttribution::for_turn(
                            new_execution_id(),
                            ctx.session_id,
                            ctx.thread_id,
                            ctx.turn_id,
                        ),
                        CancellationToken::new(),
                    ),
                );
                let _ = ctx.context.build(input).await;
            }
            Self::Tool => {
                let call = ToolCall::new(
                    "unexpected",
                    probe_tool_spec().name,
                    json!({"value": "safe"}),
                );
                let _ = ToolOrchestrator::default().execute(&ctx, &task, call).await;
            }
            Self::Exposure => {
                let input =
                    ToolExposureInput::new(ToolExposureRequest::new(task), vec![probe_tool_spec()]);
                let _ = ctx.tool_exposure.select(input).await;
            }
            Self::Compaction => {
                let input = CompactionInput::new(
                    task,
                    proteus_contracts::model_standard::CanonicalModelRequest::new(
                        ctx.model_ref.clone(),
                        history,
                    ),
                );
                let _ = ctx
                    .compactor
                    .compact(input, Arc::new(RuntimeCompactionHost::new(ctx.clone())))
                    .await;
            }
        }
        anyhow::bail!(FAILURE)
    }
}

async fn replay_probe(probe: Probe) -> WorkflowReplayReport {
    let journal =
        terminal_journal(TurnSettlementStatus::Error, TerminalModel::Absent, FAILURE).await;
    let mut catalog = ModuleCatalog::new();
    catalog.register_test_workflow(WORKFLOW_ID, Arc::new(probe));
    catalog.register_test_policy(POLICY_ID, Arc::new(ReplayAllowAll));
    replay_workflow(
        journal.store.session_dir(),
        &AppConfig::default(),
        &catalog,
        WorkflowReplayOptions::default(),
    )
    .await
    .expect("model-free replay")
}

#[tokio::test]
async fn terminal_error_before_any_model_or_tool_call_replays() {
    let report = replay_probe(Probe::NoCalls).await;
    assert!(report.comparison.matched, "{:?}", report.comparison.issues);
    assert_eq!(report.replay.status, TurnSettlementStatus::Error);
    assert_eq!(report.comparison.error_equal, Some(true));
    assert_eq!(report.comparison.history_equal, Some(true));
    assert_eq!(report.model_exchanges.recorded, 0);
    assert_eq!(report.model_exchanges.replayed, 0);
    assert_eq!(report.tool_calls.recorded, 0);
    assert_eq!(report.tool_calls.replayed, 0);
    assert!(report.source_journal_unchanged);
}

async fn assert_caught_call_diverges(probe: Probe, issue: &str) {
    let report = replay_probe(probe).await;
    assert_eq!(report.comparison.error_equal, Some(true));
    assert!(!report.comparison.matched);
    assert!(
        report
            .comparison
            .issues
            .iter()
            .any(|message| message.contains(issue)),
        "{:?}",
        report.comparison.issues
    );
    assert!(report.source_journal_unchanged);
}

#[tokio::test]
async fn caught_unrecorded_model_call_still_diverges() {
    assert_caught_call_diverges(Probe::Model, "unexpected model request #1").await;
}

#[tokio::test]
async fn caught_unrecorded_tool_call_still_diverges() {
    assert_caught_call_diverges(Probe::Tool, "unexpected replay tool call").await;
}

#[tokio::test]
async fn caught_unavailable_context_still_diverges() {
    assert_caught_call_diverges(Probe::Context, "no replayable context bundle").await;
}

#[tokio::test]
async fn caught_unavailable_exposure_still_diverges() {
    assert_caught_call_diverges(
        Probe::Exposure,
        "without a remaining recorded model request",
    )
    .await;
}

#[tokio::test]
async fn caught_unavailable_compaction_still_diverges() {
    assert_caught_call_diverges(
        Probe::Compaction,
        "without a remaining recorded model request",
    )
    .await;
}

#[tokio::test]
async fn incomplete_model_exchange_is_not_a_model_free_turn() {
    let journal =
        terminal_journal(TurnSettlementStatus::Error, TerminalModel::Pending, FAILURE).await;
    let error = replay_workflow(
        journal.store.session_dir(),
        &AppConfig::default(),
        &catalog(false),
        WorkflowReplayOptions::default(),
    )
    .await
    .expect_err("incomplete model request must remain unsupported");
    assert!(
        error
            .to_string()
            .contains("is incomplete and cannot be used for workflow replay"),
        "{error:#}"
    );
}
