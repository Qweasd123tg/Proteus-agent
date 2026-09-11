use super::*;
use crate::{
    contracts::ModelCallOrigin,
    core::{
        ModelRequestRecorded, ModelResponseOutcome, ModelResponseRecorded, TurnOpened, TurnSettled,
        TurnSettlementStatus,
    },
    domain::{
        AgentTask, ModelRef, ModelUsageStatus, new_exchange_id, new_execution_id, new_session_id,
        new_thread_id, new_turn_id,
    },
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, FinishReason, ModelFailure, TokenUsage,
    },
};

#[tokio::test]
async fn usage_tracks_exchanges_and_matches_read_only_cold_journal() {
    let config = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let store = SessionStore::new(config.path(), workspace.path(), new_session_id()).unwrap();
    assert!(store.usage_snapshot().await.unwrap().requests.is_empty());
    assert!(
        !store.session_dir().exists(),
        "reading an empty session must not materialize it"
    );
    let thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let attribution =
        ExecutionAttribution::for_turn(new_execution_id(), store.session_id(), thread_id, turn_id);
    store
        .append_execution_journal_entry(
            attribution,
            JournalEntry::TurnOpened(TurnOpened {
                intent: None,
                task: AgentTask::new("usage", workspace.path().into()),
                base_history_revision: 0,
                module_epoch: 0,
                config_snapshot: serde_json::json!({}),
            }),
        )
        .await
        .unwrap();
    let mut exchanges = Vec::new();
    for (model, origin) in [
        ("first", ModelCallOrigin::Direct),
        ("first", ModelCallOrigin::Direct),
        ("summary", ModelCallOrigin::Compactor),
        ("unknown", ModelCallOrigin::Direct),
    ] {
        let exchange_id = new_exchange_id();
        exchanges.push(exchange_id);
        store
            .append_execution_journal_entry(
                attribution,
                JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                    exchange_id,
                    origin,
                    request: CanonicalModelRequest::new(
                        ModelRef::new("fixture", model),
                        vec![CanonicalMessage::text(MessageRole::User, "private prompt")],
                    ),
                }),
            )
            .await
            .unwrap();
    }
    for (index, outcome) in [
        (
            0,
            ModelResponseOutcome::Response {
                response: CanonicalModelResponse::new(
                    CanonicalMessage::text(MessageRole::Assistant, "private answer"),
                    Vec::new(),
                    FinishReason::Stop,
                )
                .with_usage(
                    TokenUsage::new(1000, 200)
                        .with_cached_input_tokens(Some(800))
                        .with_cache_creation_input_tokens(Some(100))
                        .with_reasoning_output_tokens(Some(80)),
                ),
            },
        ),
        (
            1,
            ModelResponseOutcome::Error {
                failure: ModelFailure::other("request failed"),
            },
        ),
        (
            2,
            ModelResponseOutcome::Response {
                response: CanonicalModelResponse::new(
                    CanonicalMessage::text(MessageRole::Assistant, "summary"),
                    Vec::new(),
                    FinishReason::Stop,
                )
                .with_usage(TokenUsage::new(50, 10)),
            },
        ),
    ] {
        store
            .append_execution_journal_entry(
                attribution,
                JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                    exchange_id: exchanges[index],
                    outcome,
                }),
            )
            .await
            .unwrap();
    }
    store
        .append_journal_entry(
            thread_id,
            Some(turn_id),
            JournalEntry::TurnSettled(TurnSettled {
                status: TurnSettlementStatus::Canceled,
                output: None,
                error: Some("cancelled".into()),
            }),
        )
        .await
        .unwrap();
    let warm = store.usage_snapshot().await.unwrap();
    assert_eq!(warm.requests.len(), 4);
    assert_eq!(warm.latest_turn_id, Some(turn_id));
    assert_eq!(
        warm.requests
            .iter()
            .map(|request| request.exchange_id)
            .collect::<Vec<_>>(),
        exchanges
    );
    assert_eq!(
        warm.requests[0].usage.as_ref().unwrap().cached_input_tokens,
        Some(800)
    );
    assert_eq!(warm.requests[1].status, ModelUsageStatus::Error);
    assert_eq!(warm.requests[2].origin, ModelCallOrigin::Compactor);
    assert_eq!(warm.requests[3].status, ModelUsageStatus::Canceled);
    assert!(warm.requests[1].usage.is_none() && warm.requests[3].usage.is_none());
    let json = serde_json::to_string(&warm).unwrap();
    assert!(!json.contains("private prompt") && !json.contains("private answer"));
    let before = std::fs::read(store.journal_path()).unwrap();
    let mut cold = SessionStore::open(store.session_dir().into()).unwrap();
    cold.writer = Arc::new(Mutex::new(JournalWriterState::default()));
    assert_eq!(cold.usage_snapshot().await.unwrap(), warm);
    assert_eq!(
        store.usage_snapshot().await.unwrap(),
        warm,
        "repeated reads must not accumulate twice"
    );
    assert_eq!(std::fs::read(store.journal_path()).unwrap(), before);
}
