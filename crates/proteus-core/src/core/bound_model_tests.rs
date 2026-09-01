use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::Result;
use async_trait::async_trait;
use futures_util::stream;
use tokio::sync::{Barrier, Mutex};

use super::*;
use crate::{
    contracts::{
        CancellationToken, EventSink, ExecutionAttribution, ExecutionRecorder,
        NoopExecutionRecorder,
    },
    core::{JournalEntry, SessionExecutionRecorder, SessionStore, TurnOpened},
    domain::{AgentTask, EventEnvelope, new_session_id, new_thread_id, new_turn_id},
    model_standard::{CanonicalMessage, ContentPart, FinishReason, MessageRole},
};

#[derive(Default)]
struct CollectingSink {
    events: Mutex<Vec<EventEnvelope>>,
}

#[derive(Default)]
struct RecordedModelFacts {
    requests: Vec<(ExchangeId, CanonicalModelRequest)>,
    responses: Vec<(ExchangeId, CanonicalModelResponse)>,
    errors: Vec<(ExchangeId, String)>,
}

#[derive(Default)]
struct CollectingExecutionRecorder {
    facts: Mutex<RecordedModelFacts>,
}

#[async_trait]
impl ExecutionRecorder for CollectingExecutionRecorder {
    async fn model_request_recorded(
        &self,
        exchange_id: ExchangeId,
        request: &CanonicalModelRequest,
    ) -> Result<()> {
        self.facts
            .lock()
            .await
            .requests
            .push((exchange_id, request.clone()));
        Ok(())
    }

    async fn model_response_recorded(
        &self,
        exchange_id: ExchangeId,
        response: &CanonicalModelResponse,
    ) -> Result<()> {
        self.facts
            .lock()
            .await
            .responses
            .push((exchange_id, response.clone()));
        Ok(())
    }

    async fn model_error_recorded(&self, exchange_id: ExchangeId, message: &str) -> Result<()> {
        self.facts
            .lock()
            .await
            .errors
            .push((exchange_id, message.to_owned()));
        Ok(())
    }
}

#[async_trait]
impl EventSink for CollectingSink {
    async fn append(&self, envelope: EventEnvelope) -> Result<()> {
        self.events.lock().await.push(envelope);
        Ok(())
    }
}

struct ImmediateAdapter {
    requests: std::sync::Mutex<Vec<CanonicalModelRequest>>,
}

impl ImmediateAdapter {
    fn new() -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Model for ImmediateAdapter {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "immediate".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        self.requests.lock().unwrap().push(request);
        Ok(Box::pin(stream::iter(vec![Ok(
            ModelStreamEvent::Response {
                response: response("ok"),
            },
        )])))
    }
}

struct ConcurrentAdapter {
    provider_barrier: Arc<Barrier>,
    requests: Mutex<Vec<CanonicalModelRequest>>,
}

#[async_trait]
impl Model for ConcurrentAdapter {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "concurrent".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let case = request.metadata["case"]
            .as_str()
            .expect("test request case")
            .to_owned();
        self.requests.lock().await.push(request);
        self.provider_barrier.wait().await;
        Ok(Box::pin(stream::iter(vec![
            Ok(ModelStreamEvent::TextDelta { text: case.clone() }),
            Ok(ModelStreamEvent::Response {
                response: response(&case),
            }),
        ])))
    }
}

struct CancellationAdapter {
    provider_barrier: Arc<Barrier>,
}

#[async_trait]
impl Model for CancellationAdapter {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "cancellation".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let case = request.metadata["case"]
            .as_str()
            .expect("test request case")
            .to_owned();
        self.provider_barrier.wait().await;
        if case == "A" {
            Ok(Box::pin(stream::pending()))
        } else {
            Ok(Box::pin(stream::iter(vec![Ok(
                ModelStreamEvent::Response {
                    response: response(&case),
                },
            )])))
        }
    }
}

fn request(model: &str, case: &str) -> CanonicalModelRequest {
    CanonicalModelRequest::new(
        ModelRef::new(model, "test"),
        vec![CanonicalMessage::text(MessageRole::User, case)],
    )
    .with_metadata(serde_json::json!({ "case": case }))
}

fn response(text: &str) -> CanonicalModelResponse {
    CanonicalModelResponse::new(
        CanonicalMessage::text(MessageRole::Assistant, text),
        Vec::new(),
        FinishReason::Stop,
    )
}

fn response_text(response: &CanonicalModelResponse) -> &str {
    match &response.messages[0].parts[0].payload {
        ContentPart::Text { text } => text,
        other => panic!("unexpected response part: {other:?}"),
    }
}

fn detached_model(service: Arc<ModelService>) -> (ExecutionScope, BoundModel) {
    let scope = ExecutionScope::fresh(CancellationToken::new());
    let model = BoundModel::new(service, ModelExecutionBinding::detached(scope.clone()));
    (scope, model)
}

#[tokio::test]
async fn bound_model_constructs_without_turn() {
    let adapter = Arc::new(ImmediateAdapter::new());
    let service = Arc::new(ModelService::new(adapter));
    let (scope, model) = detached_model(service);

    let result = model
        .complete(request("immediate", "detached"))
        .await
        .unwrap();

    assert_eq!(response_text(&result), "ok");
    assert_eq!(model.binding().scope().execution_id, scope.execution_id);
}

#[tokio::test]
async fn detached_bound_model_records_lifecycle_without_chat_identity() {
    let adapter = Arc::new(ImmediateAdapter::new());
    let service = Arc::new(ModelService::new(adapter));
    let scope = ExecutionScope::fresh(CancellationToken::new());
    let recorder = Arc::new(CollectingExecutionRecorder::default());
    let model = BoundModel::new(
        service,
        ModelExecutionBinding::with_recorder(scope.clone(), recorder.clone()),
    );

    let result = model
        .complete(request("immediate", "detached-recording"))
        .await
        .unwrap();

    assert_eq!(response_text(&result), "ok");
    let facts = recorder.facts.lock().await;
    assert_eq!(facts.requests.len(), 1);
    assert_eq!(facts.responses.len(), 1);
    assert!(facts.errors.is_empty());
    assert_eq!(facts.requests[0].0, facts.responses[0].0);
    assert_eq!(model.binding().scope().execution_id, scope.execution_id);
}

#[tokio::test]
async fn reserved_turn_metadata_cannot_override_binding() {
    let adapter = Arc::new(ImmediateAdapter::new());
    let service = Arc::new(ModelService::new(adapter.clone()));
    let session_id = new_session_id();
    let binding = ModelExecutionBinding::for_turn(
        ExecutionScope::fresh(CancellationToken::new()),
        Arc::new(EventEmitter::new(Arc::new(CollectingSink::default()))),
        session_id,
        new_thread_id(),
        new_turn_id(),
        Arc::new(NoopExecutionRecorder),
    );
    let model = BoundModel::new(service, binding);
    let mut metadata = BTreeMap::new();
    metadata.insert("session_id".to_owned(), new_session_id().to_string());

    let error = model
        .complete(request("immediate", "mismatch").with_client_metadata(metadata))
        .await
        .expect_err("request must not override immutable binding");

    assert!(
        error.to_string().contains("conflicts with bound value"),
        "{error:#}"
    );
    assert!(adapter.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn concurrent_bound_models_keep_metadata_events_and_journal_attribution_isolated() {
    let config_dir = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let session_id = new_session_id();
    let store = SessionStore::new(config_dir.path(), workspace.path(), session_id).unwrap();
    let thread_a = new_thread_id();
    let thread_b = new_thread_id();
    let turn_a = new_turn_id();
    let turn_b = new_turn_id();
    let scope_a = ExecutionScope::fresh(CancellationToken::new());
    let scope_b = ExecutionScope::fresh(CancellationToken::new());
    let execution_a = scope_a.execution_id;
    let execution_b = scope_b.execution_id;
    for (execution_id, thread_id, turn_id, case) in [
        (execution_a, thread_a, turn_a, "A"),
        (execution_b, thread_b, turn_b, "B"),
    ] {
        store
            .append_execution_journal_entry(
                ExecutionAttribution::for_turn(execution_id, session_id, thread_id, turn_id),
                JournalEntry::TurnOpened(TurnOpened {
                    task: AgentTask::new(case, workspace.path().to_path_buf()),
                    base_history_revision: 0,
                    module_epoch: 0,
                    config_snapshot: serde_json::json!({}),
                }),
            )
            .await
            .unwrap();
    }

    let sink = Arc::new(CollectingSink::default());
    let events = Arc::new(EventEmitter::new(sink.clone()));
    let adapter = Arc::new(ConcurrentAdapter {
        provider_barrier: Arc::new(Barrier::new(2)),
        requests: Mutex::new(Vec::new()),
    });
    let service = Arc::new(ModelService::new(adapter.clone()));
    let model_a = BoundModel::new(
        service.clone(),
        ModelExecutionBinding::for_turn(
            scope_a,
            events.clone(),
            session_id,
            thread_a,
            turn_a,
            Arc::new(SessionExecutionRecorder::for_turn(
                store.clone(),
                execution_a,
                thread_a,
                turn_a,
            )),
        ),
    );
    let model_b = BoundModel::new(
        service,
        ModelExecutionBinding::for_turn(
            scope_b,
            events,
            session_id,
            thread_b,
            turn_b,
            Arc::new(SessionExecutionRecorder::for_turn(
                store.clone(),
                execution_b,
                thread_b,
                turn_b,
            )),
        ),
    );

    let (result_a, result_b) = tokio::join!(
        model_a.complete(request("concurrent", "A")),
        model_b.complete(request("concurrent", "B")),
    );
    assert_eq!(response_text(&result_a.unwrap()), "A");
    assert_eq!(response_text(&result_b.unwrap()), "B");

    let requests = adapter.requests.lock().await;
    assert_eq!(requests.len(), 2);
    for captured in requests.iter() {
        let case = captured.metadata["case"].as_str().unwrap();
        let (thread_id, turn_id) = if case == "A" {
            (thread_a, turn_a)
        } else {
            (thread_b, turn_b)
        };
        assert_eq!(
            captured.client_metadata["session_id"],
            session_id.to_string()
        );
        assert_eq!(captured.client_metadata["thread_id"], thread_id.to_string());
        assert_eq!(captured.client_metadata["turn_id"], turn_id.to_string());
    }
    drop(requests);

    let captured_events = sink.events.lock().await;
    assert_eq!(captured_events.len(), 2);
    for envelope in captured_events.iter() {
        let Event::AssistantTextDelta { text } = &envelope.event else {
            panic!("unexpected event: {:?}", envelope.event);
        };
        let (thread_id, turn_id) = if text == "A" {
            (thread_a, turn_a)
        } else {
            (thread_b, turn_b)
        };
        assert_eq!(envelope.session_id, session_id);
        assert_eq!(envelope.thread_id, thread_id);
        assert_eq!(envelope.turn_id, Some(turn_id));
    }
    drop(captured_events);

    let records = store.load_records().unwrap();
    let model_requests = records
        .iter()
        .filter_map(|record| match &record.entry {
            JournalEntry::ModelRequestRecorded(request) => Some((record, request)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(model_requests.len(), 2);
    for (record, recorded) in model_requests {
        assert_eq!(
            recorded.request.client_metadata["thread_id"],
            record
                .thread_id
                .expect("agent thread attribution")
                .to_string()
        );
        assert_eq!(
            recorded.request.client_metadata["turn_id"],
            record.turn_id.unwrap().to_string()
        );
    }
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.entry, JournalEntry::ModelResponseRecorded(_)))
            .count(),
        2
    );
    store
        .load_projection()
        .expect("concurrent journal remains a valid projection");
}

#[tokio::test]
async fn canceling_one_bound_model_does_not_cancel_its_sibling() {
    let provider_barrier = Arc::new(Barrier::new(3));
    let service = Arc::new(ModelService::new(Arc::new(CancellationAdapter {
        provider_barrier: provider_barrier.clone(),
    })));
    let (scope_a, model_a) = detached_model(service.clone());
    let (scope_b, model_b) = detached_model(service);
    let task_a = tokio::spawn(async move { model_a.complete(request("cancellation", "A")).await });
    let task_b = tokio::spawn(async move { model_b.complete(request("cancellation", "B")).await });

    provider_barrier.wait().await;
    scope_a.cancellation.cancel();

    let result_a = tokio::time::timeout(Duration::from_secs(2), task_a)
        .await
        .expect("canceled call terminates")
        .unwrap();
    let result_b = tokio::time::timeout(Duration::from_secs(2), task_b)
        .await
        .expect("sibling call terminates")
        .unwrap();
    assert!(
        result_a
            .unwrap_err()
            .to_string()
            .contains("model execution canceled")
    );
    assert_eq!(response_text(&result_b.unwrap()), "B");
    assert!(scope_a.cancellation.is_cancelled());
    assert!(!scope_b.cancellation.is_cancelled());
}
