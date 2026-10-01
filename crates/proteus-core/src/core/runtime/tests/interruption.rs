use super::*;
use crate::contracts::{WorkflowHistoryCheckpoint, WorkflowHistoryUpdate};

struct InterruptionProbeWorkflow;

#[async_trait]
impl Workflow for InterruptionProbeWorkflow {
    async fn run(
        &self,
        task: AgentTask,
        history: Vec<CanonicalMessage>,
        ctx: crate::contracts::WorkflowInvocationContext,
    ) -> Result<WorkflowOutput> {
        let ctx = ctx.into_agent()?;
        match task.text.as_str() {
            "cancel" => {
                assert!(ctx.interrupted_turns.is_empty());
                ctx.execution.scope.cancellation.cancel();
                Err(TurnAbort::Canceled.into())
            }
            "compact" | "compact_without_checkpoint" => {
                assert_eq!(ctx.interrupted_turns.len(), 1);
                assert_eq!(ctx.interrupted_turns[0].after_message_id, history[0].id);
                let mut report = HistoryCompactionReport::unchanged(history.len(), None);
                report.changed = true;
                // Retain the exact canceled anchor. Retirement must follow the
                // accepted compaction fact, not an accidental missing identity.
                if task.text == "compact" {
                    ctx.history_recorder
                        .checkpoint(WorkflowHistoryCheckpoint {
                            history: WorkflowHistoryUpdate {
                                new_messages: Vec::new(),
                                history_replacement: Some(history.clone()),
                                compactions: vec![report.clone()],
                            },
                            tool_results: Vec::new(),
                        })
                        .await?;
                }
                Ok(successful_messages(history.clone(), task, "compacted")
                    .with_history_replacement(history)
                    .with_compactions(vec![report]))
            }
            "verify" => {
                assert!(
                    ctx.interrupted_turns.is_empty(),
                    "accepted compaction must retire the interruption fact"
                );
                assert_eq!(message_text_for_test(&history[0]), "cancel");
                Ok(successful_messages(history, task, "done"))
            }
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn in_memory_interruption_is_retired_after_compaction_retaining_its_anchor() {
    for persist in [false, true] {
        for checkpoint in [false, true] {
            let cwd = tempfile::tempdir().unwrap();
            let config_path = cwd.path().join("config.toml");
            let builder = AgentRuntime::builder(AppConfig::default(), cwd.path().to_owned())
                .with_config_path(persist.then_some(config_path.as_path()))
                .with_module_catalog(test_catalog());
            let runtime = builder.build().unwrap();
            assert_eq!(runtime.session_dir().is_some(), persist);
            replace_workflow_for_test(&runtime, Arc::new(InterruptionProbeWorkflow)).await;
            runtime
                .run("cancel".to_owned())
                .await
                .expect_err("canceled turn");
            runtime
                .run(
                    if checkpoint {
                        "compact"
                    } else {
                        "compact_without_checkpoint"
                    }
                    .to_owned(),
                )
                .await
                .unwrap();
            runtime.run("verify".to_owned()).await.unwrap();
            if let Some(store) = &runtime.session.session_store {
                let projection = store.load_projection().unwrap();
                assert!(projection.interrupted_turns.is_empty());
                let compactions = projection
                    .records
                    .iter()
                    .filter(|record| {
                        matches!(&record.entry, crate::core::JournalEntry::HistoryMutated(mutation)
                if mutation.compaction.as_ref().is_some_and(|report| report.changed))
                    })
                    .count();
                assert_eq!(
                    compactions, 1,
                    "changed compaction must persist once, including an unchanged durable prefix"
                );
            }
            assert!(
                runtime
                    .history()
                    .await
                    .iter()
                    .all(|message| { !message_text_for_test(message).contains("<turn_aborted>") }),
                "generic Core must never manufacture Codex prompt text"
            );
        }
    }
}
