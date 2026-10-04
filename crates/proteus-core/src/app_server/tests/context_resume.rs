use super::*;
use crate::{
    contracts::{
        ExecutionAttribution, ModelCallOrigin, ModelContextObservation, Workflow, WorkflowOutput,
    },
    core::{
        JournalEntry, ModelRequestRecorded, ModelResponseOutcome, ModelResponseRecorded,
        PreparedAssembly, SessionConfigSnapshot, TurnOpened, TurnSettled, TurnSettlementStatus,
    },
    domain::{
        AgentOutput, AgentTask, HistoryCompactionReport, new_exchange_id, new_execution_id,
        new_thread_id, new_turn_id,
    },
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, FinishReason, ModelFailure,
        ModelFailureKind, TokenUsage,
    },
};
use async_trait::async_trait;

struct ContextProbe {
    old_thread: crate::domain::ThreadId,
    seen: Arc<std::sync::Mutex<Vec<Vec<ModelContextObservation>>>>,
}
#[async_trait]
impl Workflow for ContextProbe {
    async fn run(
        &self,
        _: AgentTask,
        _: Vec<CanonicalMessage>,
        ctx: crate::contracts::WorkflowInvocationContext,
    ) -> Result<WorkflowOutput> {
        let ctx = ctx.into_agent()?;
        assert_ne!(
            ctx.thread_id, self.old_thread,
            "actual resume creates a new thread"
        );
        self.seen.lock().unwrap().push(ctx.model_context);
        Ok(WorkflowOutput::new(
            AgentOutput::text("probed"),
            vec![CanonicalMessage::text(MessageRole::Assistant, "probed")],
        ))
    }
}

async fn resume_and_probe(
    store: &SessionStore,
    config_path: &Path,
    old_thread: crate::domain::ThreadId,
) -> Vec<ModelContextObservation> {
    let handle = AgentAppServer::launch_resumed(
        crate::test_model::config(),
        store.workspace_path().unwrap(),
        Some(config_path),
        store.session_dir().to_path_buf(),
    )
    .await
    .unwrap();
    let config = AppConfig::default();
    let mut assembly = PreparedAssembly::from_catalog(
        config.clone(),
        store.workspace_path().unwrap(),
        Some(config_path),
        test_catalog(),
    )
    .unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    assembly.registry_mut().workflow = Arc::new(ContextProbe {
        old_thread,
        seen: seen.clone(),
    });
    let snapshot = SessionConfigSnapshot::from_runtime_config(
        &config,
        assembly.registry(),
        config.permissions.mode,
    );
    handle
        .runtime
        .reload_assembly(assembly, Some(snapshot))
        .await
        .unwrap();
    handle.runtime.run("inspect context".into()).await.unwrap();
    handle.shutdown().await;
    let snapshot = seen.lock().unwrap()[0].clone();
    snapshot
}

#[tokio::test]
async fn actual_app_server_resume_restores_root_context_across_threads_and_respects_resets() {
    let root = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let config_path = root.path().join("config.toml");
    let store = SessionStore::new(root.path(), workspace.path(), new_session_id()).unwrap();
    let thread = new_thread_id();
    let turn = new_turn_id();
    let attribution =
        ExecutionAttribution::for_turn(new_execution_id(), store.session_id(), thread, turn);
    store
        .append_history(
            thread,
            None,
            &[CanonicalMessage::text(MessageRole::User, "old history")],
        )
        .await
        .unwrap();
    store
        .append_execution_journal_entry(
            attribution,
            JournalEntry::TurnOpened(TurnOpened {
                task: AgentTask::new("old history", workspace.path().into()),
                intent: None,
                base_history_revision: 1,
                module_epoch: 0,
                config_snapshot: serde_json::json!({}),
            }),
        )
        .await
        .unwrap();
    for (owner, tokens, overflow) in [
        (attribution, 12, false),
        (
            ExecutionAttribution::for_turn(
                attribution.execution_id,
                store.session_id(),
                new_thread_id(),
                turn,
            ),
            999,
            false,
        ),
        (
            ExecutionAttribution::detached(new_execution_id()),
            888,
            false,
        ),
        (attribution, 32_000, true),
    ] {
        let exchange_id = new_exchange_id();
        let mut request =
            CanonicalModelRequest::new(crate::domain::ModelRef::new("fixture", "model"), vec![]);
        request.limits.max_input_tokens = Some(tokens);
        store
            .append_execution_journal_entry(
                owner,
                JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                    exchange_id,
                    origin: ModelCallOrigin::Direct,
                    request,
                }),
            )
            .await
            .unwrap();
        let outcome = if overflow {
            ModelResponseOutcome::Error {
                failure: ModelFailure::new(ModelFailureKind::ContextWindowExceeded, "overflow"),
            }
        } else {
            let mut response =
                CanonicalModelResponse::from_messages(vec![], vec![], FinishReason::Stop);
            response.usage = Some(TokenUsage::new(tokens, 0));
            ModelResponseOutcome::Response { response }
        };
        store
            .append_execution_journal_entry(
                owner,
                JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                    exchange_id,
                    outcome,
                }),
            )
            .await
            .unwrap();
    }
    store
        .append_journal_entry(
            thread,
            Some(turn),
            JournalEntry::TurnSettled(TurnSettled {
                status: TurnSettlementStatus::Error,
                output: None,
                error: Some("overflow".into()),
            }),
        )
        .await
        .unwrap();
    let expected = vec![
        ModelContextObservation::Usage {
            total_tokens: 12,
            last_tokens: 12,
        },
        ModelContextObservation::ContextWindowExceeded {
            max_input_tokens: 32_000,
        },
    ];
    assert_eq!(
        resume_and_probe(&store, &config_path, thread).await,
        expected
    );
    assert_eq!(
        resume_and_probe(&store, &config_path, thread).await,
        expected,
        "another cold resume preserves the same lineage"
    );
    let mut report = HistoryCompactionReport::unchanged(store.load_messages().unwrap().len(), None);
    report.changed = true;
    store
        .replace_history(
            new_thread_id(),
            None,
            &store.load_messages().unwrap(),
            Some(report),
        )
        .await
        .unwrap();
    let mut compacted = expected;
    compacted.push(ModelContextObservation::HistoryCompacted);
    assert_eq!(
        resume_and_probe(&store, &config_path, thread).await,
        compacted
    );
    store.clear_history(new_thread_id()).await.unwrap();
    assert!(
        resume_and_probe(&store, &config_path, thread)
            .await
            .is_empty()
    );
}
