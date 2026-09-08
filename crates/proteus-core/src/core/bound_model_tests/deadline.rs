use super::*;
use std::{
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll},
};

use tokio::sync::Notify;

struct PendingStartAdapter;

#[async_trait]
impl Model for PendingStartAdapter {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "pending-start".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }

    async fn stream(&self, _request: CanonicalModelRequest) -> Result<ModelEventStream> {
        futures_util::future::pending().await
    }
}

struct DelayedStreamAdapter {
    start_delay: Duration,
    event_delay: Duration,
    deltas: usize,
}

#[async_trait]
impl Model for DelayedStreamAdapter {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "delayed-stream".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }

    async fn stream(&self, _request: CanonicalModelRequest) -> Result<ModelEventStream> {
        tokio::time::sleep(self.start_delay).await;
        let event_delay = self.event_delay;
        let deltas = self.deltas;
        Ok(Box::pin(async_stream::try_stream! {
            for index in 0..deltas {
                tokio::time::sleep(event_delay).await;
                yield ModelStreamEvent::TextDelta {
                    message_id: proteus_contracts::domain::new_message_id(),
                    phase: None,
                    text: index.to_string(),
                };
            }
            tokio::time::sleep(event_delay).await;
            yield ModelStreamEvent::Response {
                response: response("delayed"),
            };
        }))
    }
}

struct StalledSink;

#[async_trait]
impl EventSink for StalledSink {
    async fn append(&self, _envelope: EventEnvelope) -> Result<()> {
        futures_util::future::pending().await
    }
}

struct PendingDropStream {
    dropped: Arc<AtomicBool>,
}

impl futures_core::Stream for PendingDropStream {
    type Item = Result<ModelStreamEvent>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

impl Drop for PendingDropStream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

struct DropObservedAdapter {
    dropped: Arc<AtomicBool>,
}

#[async_trait]
impl Model for DropObservedAdapter {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "drop-observed".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }

    async fn stream(&self, _request: CanonicalModelRequest) -> Result<ModelEventStream> {
        Ok(Box::pin(PendingDropStream {
            dropped: self.dropped.clone(),
        }))
    }
}

struct BlockingErrorRecorder {
    inner: CollectingExecutionRecorder,
    error_started: Notify,
    release_error: Notify,
}

impl BlockingErrorRecorder {
    fn new() -> Self {
        Self {
            inner: CollectingExecutionRecorder::default(),
            error_started: Notify::new(),
            release_error: Notify::new(),
        }
    }
}

#[async_trait]
impl ExecutionRecorder for BlockingErrorRecorder {
    async fn model_request_recorded(
        &self,
        exchange_id: ExchangeId,
        origin: ModelCallOrigin,
        request: &CanonicalModelRequest,
    ) -> Result<()> {
        self.inner
            .model_request_recorded(exchange_id, origin, request)
            .await
    }

    async fn model_response_recorded(
        &self,
        exchange_id: ExchangeId,
        response: &CanonicalModelResponse,
    ) -> Result<()> {
        self.inner
            .model_response_recorded(exchange_id, response)
            .await
    }

    async fn model_error_recorded(
        &self,
        exchange_id: ExchangeId,
        failure: &crate::model_standard::ModelFailure,
    ) -> Result<()> {
        self.error_started.notify_one();
        self.release_error.notified().await;
        self.inner.model_error_recorded(exchange_id, failure).await
    }
}

fn recorded_model(
    adapter: Arc<dyn Model>,
    model_timeout_ms: u64,
) -> (Arc<CollectingExecutionRecorder>, BoundModel) {
    let recorder = Arc::new(CollectingExecutionRecorder::default());
    let binding = ModelExecutionBinding::with_recorder(
        ExecutionScope::fresh(CancellationToken::new()),
        recorder.clone(),
    );
    (
        recorder,
        BoundModel::new(
            Arc::new(ModelService::new(adapter)),
            binding,
            model_timeout_ms,
        ),
    )
}

async fn assert_single_timeout(recorder: &CollectingExecutionRecorder, expected_message: &str) {
    let facts = recorder.facts.lock().await;
    assert_eq!(facts.requests.len(), 1);
    assert!(facts.responses.is_empty());
    assert_eq!(facts.errors.len(), 1);
    assert_eq!(facts.requests[0].0, facts.errors[0].0);
    assert_eq!(facts.errors[0].1, expected_message);
}

#[tokio::test]
async fn model_deadline_records_start_timeout_for_same_exchange() {
    let (recorder, model) = recorded_model(Arc::new(PendingStartAdapter), 30);

    let error = model
        .complete(request("pending-start", "start-timeout"))
        .await
        .expect_err("model start must time out");

    assert_eq!(error.to_string(), "model request timed out after 30ms");
    assert!(
        error
            .downcast_ref::<crate::model_standard::ModelFailure>()
            .is_some(),
        "timeout remains a typed model failure"
    );
    assert_single_timeout(&recorder, "model request timed out after 30ms").await;
}

#[tokio::test]
async fn model_deadline_records_stream_timeout_for_same_exchange() {
    let adapter = DelayedStreamAdapter {
        start_delay: Duration::ZERO,
        event_delay: Duration::from_millis(200),
        deltas: 0,
    };
    let (recorder, model) = recorded_model(Arc::new(adapter), 30);

    let error = model
        .complete(request("delayed-stream", "stream-timeout"))
        .await
        .expect_err("model stream must time out");

    assert_eq!(error.to_string(), "model request timed out after 30ms");
    assert_single_timeout(&recorder, "model request timed out after 30ms").await;
}

#[tokio::test]
async fn continuous_stream_uses_one_total_deadline() {
    let adapter = DelayedStreamAdapter {
        start_delay: Duration::from_millis(10),
        event_delay: Duration::from_millis(20),
        deltas: 5,
    };
    let (recorder, model) = recorded_model(Arc::new(adapter), 80);

    let error = model
        .complete(request("delayed-stream", "total-budget"))
        .await
        .expect_err("continuous deltas must not reset the model deadline");

    assert_eq!(error.to_string(), "model request timed out after 80ms");
    assert_single_timeout(&recorder, "model request timed out after 80ms").await;
}

#[tokio::test]
async fn zero_model_timeout_disables_deadline() {
    let adapter = DelayedStreamAdapter {
        start_delay: Duration::from_millis(20),
        event_delay: Duration::from_millis(20),
        deltas: 1,
    };
    let (recorder, model) = recorded_model(Arc::new(adapter), 0);

    let response = model
        .complete(request("delayed-stream", "disabled"))
        .await
        .unwrap();

    assert_eq!(response_text(&response), "delayed");
    let facts = recorder.facts.lock().await;
    assert_eq!(facts.requests.len(), 1);
    assert_eq!(facts.responses.len(), 1);
    assert!(facts.errors.is_empty());
    assert_eq!(facts.requests[0].0, facts.responses[0].0);
}

#[tokio::test]
async fn stalled_delta_sink_still_allows_stream_deadline_to_record() {
    let recorder = Arc::new(CollectingExecutionRecorder::default());
    let binding = ModelExecutionBinding::for_turn(
        ExecutionScope::fresh(CancellationToken::new()),
        Arc::new(EventEmitter::new(Arc::new(StalledSink))),
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        recorder.clone(),
    );
    let adapter = Arc::new(DelayedStreamAdapter {
        start_delay: Duration::ZERO,
        event_delay: Duration::from_millis(1),
        deltas: 10,
    });
    let model = BoundModel::new(Arc::new(ModelService::new(adapter)), binding, 30);

    let error = model
        .complete(request("delayed-stream", "stalled-sink"))
        .await
        .expect_err("presentation sink must not hide the model deadline");

    assert_eq!(error.to_string(), "model request timed out after 30ms");
    assert_single_timeout(&recorder, "model request timed out after 30ms").await;
}

#[tokio::test]
async fn terminal_response_is_not_replaced_when_final_sink_stalls() {
    let recorder = Arc::new(CollectingExecutionRecorder::default());
    let binding = ModelExecutionBinding::for_turn(
        ExecutionScope::fresh(CancellationToken::new()),
        Arc::new(EventEmitter::new(Arc::new(StalledSink))),
        new_session_id(),
        new_thread_id(),
        new_turn_id(),
        recorder.clone(),
    );
    let model = BoundModel::new(
        Arc::new(ModelService::new(Arc::new(ImmediateAdapter::new()))),
        binding,
        30,
    );

    let result = model
        .complete(request("immediate", "terminal-sink"))
        .await
        .unwrap();

    assert_eq!(response_text(&result), "ok");
    let facts = recorder.facts.lock().await;
    assert_eq!(facts.requests.len(), 1);
    assert_eq!(facts.responses.len(), 1);
    assert!(facts.errors.is_empty());
    assert_eq!(facts.requests[0].0, facts.responses[0].0);
}

#[tokio::test]
async fn deadline_drops_provider_stream_before_waiting_for_error_recording() {
    let dropped = Arc::new(AtomicBool::new(false));
    let recorder = Arc::new(BlockingErrorRecorder::new());
    let binding = ModelExecutionBinding::with_recorder(
        ExecutionScope::fresh(CancellationToken::new()),
        recorder.clone(),
    );
    let model = BoundModel::new(
        Arc::new(ModelService::new(Arc::new(DropObservedAdapter {
            dropped: dropped.clone(),
        }))),
        binding,
        30,
    );

    let completion = tokio::spawn(async move {
        model
            .complete(request("drop-observed", "drop-before-record"))
            .await
    });
    recorder.error_started.notified().await;
    assert!(
        dropped.load(Ordering::SeqCst),
        "provider stream must be dropped before recorder await"
    );
    recorder.release_error.notify_one();

    let error = completion
        .await
        .unwrap()
        .expect_err("deadline must fail completion");
    assert_eq!(error.to_string(), "model request timed out after 30ms");
    let facts = recorder.inner.facts.lock().await;
    assert_eq!(facts.requests.len(), 1);
    assert_eq!(facts.errors.len(), 1);
    assert_eq!(facts.requests[0].0, facts.errors[0].0);
}
