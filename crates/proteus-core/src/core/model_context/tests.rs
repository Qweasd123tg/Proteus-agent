use super::*;
use crate::{
    contracts::{ExecutionRecorder, NoopExecutionRecorder},
    core::{ModelRequestRecorded, ModelResponseRecorded, TurnOpened},
    domain::{
        AgentTask, ModelRef, new_exchange_id, new_execution_id, new_record_id, new_session_id,
        new_thread_id, new_turn_id,
    },
    model_standard::{FinishReason, TokenUsage},
};
use std::sync::Arc;
use tokio::sync::Mutex;

fn request(window: Option<u32>) -> CanonicalModelRequest {
    let mut request = CanonicalModelRequest::new(ModelRef::new("fixture", "model"), vec![]);
    request.limits.max_input_tokens = window;
    request
}

fn overflow() -> ModelFailure {
    ModelFailure::new(ModelFailureKind::ContextWindowExceeded, "overflow")
}

#[tokio::test]
async fn memory_recorder_keeps_observations_without_inventing_usage_or_summary_overflow() {
    let context = Arc::new(Mutex::new(ModelContextState::default()));
    let recorder = recorder::ContextExecutionRecorder {
        inner: Arc::new(NoopExecutionRecorder),
        context: context.clone(),
    };
    for origin in [ModelCallOrigin::Direct, ModelCallOrigin::Compactor] {
        let exchange = new_exchange_id();
        recorder
            .model_request_recorded(exchange, origin, &request(Some(100_000)))
            .await
            .unwrap();
        let mut response =
            CanonicalModelResponse::from_messages(vec![], vec![], FinishReason::Stop);
        response.usage = Some(TokenUsage::new(10, 2));
        recorder
            .model_response_recorded(exchange, &response)
            .await
            .unwrap();
    }
    for (origin, window) in [
        (ModelCallOrigin::Compactor, Some(100_000)),
        (ModelCallOrigin::Direct, None),
        (ModelCallOrigin::Direct, Some(100_000)),
    ] {
        let exchange = new_exchange_id();
        recorder
            .model_request_recorded(exchange, origin, &request(window))
            .await
            .unwrap();
        recorder
            .model_error_recorded(exchange, &overflow())
            .await
            .unwrap();
    }
    let expected = vec![
        ModelContextObservation::Usage {
            total_tokens: 24,
            last_tokens: 12,
        },
        ModelContextObservation::ContextWindowExceeded {
            max_input_tokens: 100_000,
        },
    ];
    assert_eq!(context.lock().await.snapshot(), expected);
    let exchange = new_exchange_id();
    recorder
        .model_request_recorded(exchange, ModelCallOrigin::Direct, &request(Some(100_000)))
        .await
        .unwrap();
    recorder
        .model_response_recorded(
            exchange,
            &CanonicalModelResponse::from_messages(vec![], vec![], FinishReason::Stop),
        )
        .await
        .unwrap();
    assert_eq!(
        context.lock().await.snapshot(),
        expected,
        "response without usage preserves facts"
    );
}

#[test]
fn journal_prefix_excludes_other_threads_detached_and_nested_executions() {
    let session_id = new_session_id();
    let thread_id = new_thread_id();
    let turn_id = new_turn_id();
    let execution_id = new_execution_id();
    let next_turn = new_turn_id();
    let opened = || {
        JournalEntry::TurnOpened(TurnOpened {
            task: AgentTask::new("fixture", "/tmp".into()),
            intent: None,
            base_history_revision: 0,
            module_epoch: 0,
            config_snapshot: serde_json::json!({}),
        })
    };
    let mut records = Vec::new();
    let mut push = |thread, turn, execution, entry| {
        records.push(JournalRecord {
            schema_version: crate::core::JOURNAL_SCHEMA_VERSION,
            record_id: new_record_id(),
            session_seq: records.len() as u64 + 1,
            timestamp_ms: 0,
            session_id,
            thread_id: thread,
            turn_id: turn,
            execution_id: execution,
            entry,
        });
    };
    push(Some(thread_id), Some(turn_id), Some(execution_id), opened());
    for (thread, turn, execution, window) in [
        (Some(new_thread_id()), Some(turn_id), Some(execution_id), 10),
        (None, None, Some(new_execution_id()), 20),
        (Some(thread_id), Some(turn_id), Some(new_execution_id()), 30),
        (Some(thread_id), Some(turn_id), Some(execution_id), 100_000),
    ] {
        let exchange_id = new_exchange_id();
        push(
            thread,
            turn,
            execution,
            JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                exchange_id,
                origin: ModelCallOrigin::Direct,
                request: request(Some(window)),
            }),
        );
        push(
            thread,
            turn,
            execution,
            JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                exchange_id,
                outcome: ModelResponseOutcome::Error {
                    failure: overflow(),
                },
            }),
        );
    }
    push(
        Some(thread_id),
        Some(next_turn),
        Some(new_execution_id()),
        opened(),
    );
    push(
        Some(thread_id),
        None,
        None,
        JournalEntry::HistoryMutated(crate::core::HistoryMutated {
            previous_revision: 0,
            new_revision: 1,
            mutation: HistoryMutationKind::Replace,
            messages: vec![],
            compaction: None,
            tool_results: vec![],
        }),
    );
    let prefix = ModelContextState::from_records(&records, Some(next_turn));
    assert_eq!(
        prefix.snapshot(),
        vec![ModelContextObservation::ContextWindowExceeded {
            max_input_tokens: 100_000,
        }]
    );
    assert!(
        ModelContextState::from_records(&records, None)
            .snapshot()
            .is_empty()
    );
}
