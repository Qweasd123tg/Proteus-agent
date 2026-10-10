use super::*;
use crate::{
    contracts::{ExecutionAttribution, ModelCallOrigin},
    core::{
        ModelRequestRecorded, ModelResponseRecorded, ToolCallRecorded, ToolResultRecorded,
        TurnOpened, TurnSettled,
    },
    domain::{
        AgentOutput, AgentTask, ModelRef, ToolCall, ToolCallResolution, ToolResult,
        new_exchange_id, new_execution_id, new_session_id, new_thread_id, new_turn_id,
    },
    model_standard::{CanonicalMessage, CanonicalModelRequest, MessageRole, ModelFailure},
};
use serde_json::json;

async fn read(query: Option<&str>) -> Result<AppSessionAnalysis> {
    let (session_dir, requested) = parse(query)?;
    read_stored(session_dir, requested).await
}

async fn append(store: &SessionStore, owner: ExecutionAttribution, entry: JournalEntry) {
    if matches!(entry, JournalEntry::TurnSettled(_)) {
        let agent = owner.agent.unwrap();
        store
            .append_journal_entry(agent.thread_id, Some(agent.turn_id), entry)
            .await
            .unwrap();
    } else {
        store
            .append_execution_journal_entry(owner, entry)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn cold_analysis_preserves_historical_inputs_failures_and_unknown_effects() {
    let root = tempfile::tempdir().unwrap();
    let store = SessionStore::new(root.path(), root.path(), new_session_id()).unwrap();
    let first = new_turn_id();
    let owner = ExecutionAttribution::for_turn(
        new_execution_id(),
        store.session_id(),
        new_thread_id(),
        first,
    );
    let open = |text: &str, epoch| {
        JournalEntry::TurnOpened(TurnOpened {
            task: AgentTask::new(text, root.path().to_path_buf()),
            intent: None,
            base_history_revision: 0,
            module_epoch: epoch,
            config_snapshot: json!({"profile_name":text}),
        })
    };
    append(&store, owner, open("original", 3)).await;
    let exchange = new_exchange_id();
    let request = CanonicalModelRequest::new(
        ModelRef::new("fixture", "old-model"),
        vec![CanonicalMessage::text(MessageRole::User, "old context")],
    );
    append(
        &store,
        owner,
        JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
            exchange_id: exchange,
            origin: ModelCallOrigin::Compactor,
            request: request.clone(),
        }),
    )
    .await;
    append(
        &store,
        owner,
        JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
            exchange_id: exchange,
            outcome: ModelResponseOutcome::Error {
                failure: ModelFailure::other("summary failed"),
            },
        }),
    )
    .await;
    for id in ["finished", "unknown"] {
        let call = ToolCall::new(id, "shell", json!({"command":"append"}));
        for phase in [
            ToolCallRecordPhase::Requested,
            ToolCallRecordPhase::ApprovalRequested {
                reason: "confirm append".into(),
            },
            ToolCallRecordPhase::Resolved {
                resolution: ToolCallResolution::Approved,
            },
        ] {
            append(
                &store,
                owner,
                JournalEntry::ToolCallRecorded(ToolCallRecorded {
                    call: call.clone(),
                    phase,
                }),
            )
            .await;
        }
        if id == "finished" {
            append(
                &store,
                owner,
                JournalEntry::ToolResultRecorded(ToolResultRecorded {
                    result: ToolResult::ok(id.into(), "effect recorded"),
                }),
            )
            .await;
        }
    }
    append(
        &store,
        owner,
        JournalEntry::TurnSettled(TurnSettled {
            status: TurnSettlementStatus::Error,
            output: None,
            error: Some("root failure".into()),
        }),
    )
    .await;
    let second = new_turn_id();
    let next_owner = ExecutionAttribution::for_turn(
        new_execution_id(),
        store.session_id(),
        new_thread_id(),
        second,
    );
    append(&store, next_owner, open("new config", 4)).await;
    append(
        &store,
        next_owner,
        JournalEntry::TurnSettled(TurnSettled {
            status: TurnSettlementStatus::Success,
            output: Some(AgentOutput::text("done")),
            error: None,
        }),
    )
    .await;
    let before = std::fs::read(store.journal_path()).unwrap();
    let query = format!(
        "session_dir={}&turn_id={first}",
        store.session_dir().display()
    );
    let report = read(Some(&query)).await.unwrap();
    assert_eq!(report.turns.len(), 2);
    assert_eq!(report.turns[0].status, AppAnalysisTurnStatus::Error);
    assert_eq!(report.turns[1].status, AppAnalysisTurnStatus::Success);
    let selected = report.selected.unwrap();
    assert_eq!(selected.turn_id, first);
    assert_eq!(selected.config_snapshot["profile_name"], "original");
    assert_eq!(selected.module_epoch, 3);
    assert_eq!(selected.error.as_deref(), Some("root failure"));
    assert_eq!(
        selected.steps.len(),
        3,
        "approval phases must stay with the tool"
    );
    match &selected.steps[0].data {
        AppAnalysisStepData::Model {
            request: actual,
            origin,
            failure,
            ..
        } => {
            assert_eq!(**actual, request);
            assert_eq!(*origin, ModelCallOrigin::Compactor);
            assert_eq!(failure.as_ref().unwrap().message, "summary failed");
        }
        _ => panic!("model step"),
    }
    assert!(matches!(
        &selected.steps[1].data,
        AppAnalysisStepData::Tool {
            result: Some(_),
            ..
        }
    ));
    assert!(matches!(
        &selected.steps[2].data,
        AppAnalysisStepData::Tool {
            result: None,
            resolution: Some(ToolCallResolution::Approved),
            ..
        }
    ));
    assert_eq!(selected.steps[2].finished_at_ms, None);
    let latest = read(Some(&format!(
        "session_dir={}",
        store.session_dir().display()
    )))
    .await
    .unwrap();
    assert_eq!(latest.selected.unwrap().turn_id, second);
    assert!(
        read(Some(
            &query.replace(&first.to_string(), &new_turn_id().to_string())
        ))
        .await
        .is_err()
    );
    assert!(
        read(Some(&query.replace(&first.to_string(), "bad-id")))
            .await
            .is_err()
    );
    for invalid in [
        format!("{query}&turn_id={first}"),
        format!("session_dir={}&turn_id", store.session_dir().display()),
    ] {
        assert!(read(Some(&invalid)).await.is_err());
    }
    assert_eq!(std::fs::read(store.journal_path()).unwrap(), before);
}
