use futures_util::stream;

use super::*;
use crate::{
    domain::{ToolCall, new_part_id},
    model_standard::{CanonicalPart, MessagePhase, ModelFailure, PartProvenance, PartScope},
};

struct ProgressAdapter {
    events: std::sync::Mutex<Option<Vec<ModelStreamEvent>>>,
}

struct StartFailureAdapter {
    failure: ModelFailure,
}

#[async_trait]
impl Model for StartFailureAdapter {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "start-failure-progress".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }

    async fn stream(&self, _request: CanonicalModelRequest) -> Result<ModelEventStream> {
        Err(anyhow::Error::new(self.failure.clone()))
    }
}

struct CompletedThenPendingAdapter {
    message: CanonicalMessage,
}

#[async_trait]
impl Model for CompletedThenPendingAdapter {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "completed-then-pending".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }

    async fn stream(&self, _request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let message = self.message.clone();
        Ok(Box::pin(async_stream::try_stream! {
            yield ModelStreamEvent::MessageCompleted { message };
            futures_util::future::pending::<()>().await;
        }))
    }
}

impl ProgressAdapter {
    fn new(events: Vec<ModelStreamEvent>) -> Self {
        Self {
            events: std::sync::Mutex::new(Some(events)),
        }
    }
}

#[async_trait]
impl Model for ProgressAdapter {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        "failure-progress".into()
    }

    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        ModelCapabilities::empty()
    }

    async fn stream(&self, _request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let events = self.events.lock().unwrap().take().unwrap();
        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

fn completed(text: &str, phase: MessagePhase) -> CanonicalMessage {
    CanonicalMessage::text(MessageRole::Assistant, text).with_phase(phase)
}

fn recording_model(
    events: Vec<ModelStreamEvent>,
) -> (BoundModel, Arc<CollectingExecutionRecorder>) {
    let recorder = Arc::new(CollectingExecutionRecorder::default());
    let binding = ModelExecutionBinding::with_recorder(
        ExecutionScope::fresh(CancellationToken::new()),
        recorder.clone(),
    );
    let service = Arc::new(ModelService::new(Arc::new(ProgressAdapter::new(events))));
    (BoundModel::new(service, binding, 0), recorder)
}

#[tokio::test]
async fn completed_messages_are_attached_on_eof_even_when_deltas_are_suppressed() {
    let message = completed("durable before EOF", MessagePhase::Commentary);
    let (model, recorder) = recording_model(vec![
        ModelStreamEvent::MessageCompleted {
            message: message.clone(),
        },
        ModelStreamEvent::TextDelta {
            message_id: crate::domain::new_message_id(),
            phase: Some(MessagePhase::Commentary),
            text: "unfinished".to_owned(),
        },
    ]);
    let request = request("failure-progress", "eof").with_metadata(serde_json::json!({
        "suppress_stream_deltas": true,
    }));

    let error = model.complete(request).await.unwrap_err();
    let failure = ModelFailure::from_error(&error);
    assert_eq!(failure.completed_messages, [message.clone()]);
    let facts = recorder.facts.lock().await;
    assert_eq!(facts.errors.len(), 1);
    assert_eq!(facts.errors[0].2, [message]);
    assert!(facts.responses.is_empty());
}

#[tokio::test]
async fn embedded_failure_progress_merges_without_comparing_generated_part_ids() {
    let first = completed("first", MessagePhase::Commentary);
    let mut duplicate = first.clone();
    for part in &mut duplicate.parts {
        part.part_id = new_part_id();
    }
    let second = completed("second", MessagePhase::FinalAnswer);
    let failure = ModelFailure::other("provider stopped")
        .with_completed_messages(vec![duplicate, second.clone()]);
    let (model, recorder) = recording_model(vec![
        ModelStreamEvent::MessageCompleted {
            message: first.clone(),
        },
        ModelStreamEvent::Error { failure },
    ]);

    let error = model
        .complete(request("failure-progress", "embedded"))
        .await
        .unwrap_err();
    let failure = ModelFailure::from_error(&error);
    assert_eq!(failure.completed_messages, [first.clone(), second.clone()]);
    assert_eq!(recorder.facts.lock().await.errors[0].2, [first, second]);
}

#[tokio::test]
async fn start_failure_progress_is_validated_and_recorded() {
    let message = completed(
        "accepted by provider before start error",
        MessagePhase::Commentary,
    );
    let recorder = Arc::new(CollectingExecutionRecorder::default());
    let binding = ModelExecutionBinding::with_recorder(
        ExecutionScope::fresh(CancellationToken::new()),
        recorder.clone(),
    );
    let adapter = StartFailureAdapter {
        failure: ModelFailure::other("request failed")
            .with_completed_messages(vec![message.clone()]),
    };
    let model = BoundModel::new(Arc::new(ModelService::new(Arc::new(adapter))), binding, 0);

    let error = model
        .complete(request("start-failure-progress", "start"))
        .await
        .unwrap_err();
    assert_eq!(
        ModelFailure::from_error(&error).completed_messages,
        [message.clone()]
    );
    assert_eq!(recorder.facts.lock().await.errors[0].2, [message]);
}

#[tokio::test]
async fn invalid_embedded_progress_records_protocol_failure_with_prior_progress() {
    let first = completed("valid progress", MessagePhase::Commentary);
    let invalid_role = CanonicalMessage::text(MessageRole::User, "invalid role");
    let mut invalid_scope = completed("request-scoped progress", MessagePhase::Commentary);
    invalid_scope.parts[0].scope = PartScope::Request;
    let mut reused_part = completed("reused part id", MessagePhase::Commentary);
    reused_part.parts[0].part_id = first.parts[0].part_id;
    let mut duplicate_part = completed("duplicate part id", MessagePhase::Commentary);
    duplicate_part.parts.push(duplicate_part.parts[0].clone());
    let current_request = request("failure-progress", "invalid embedded");
    let mut request_part = completed("request part id", MessagePhase::Commentary);
    request_part.parts[0].part_id = current_request.messages[0].parts[0].part_id;
    for invalid in [
        invalid_role,
        invalid_scope,
        reused_part,
        duplicate_part,
        request_part,
    ] {
        let failure = ModelFailure::new(
            crate::model_standard::ModelFailureKind::StreamDisconnected,
            "provider stopped",
        )
        .with_completed_messages(vec![invalid]);
        let (model, recorder) = recording_model(vec![
            ModelStreamEvent::MessageCompleted {
                message: first.clone(),
            },
            ModelStreamEvent::Error { failure },
        ]);

        let error = model.complete(current_request.clone()).await.unwrap_err();
        let failure = ModelFailure::from_error(&error);
        assert!(failure.message.contains("model protocol error"));
        assert_eq!(failure.kind, crate::model_standard::ModelFailureKind::Other);
        assert_eq!(failure.completed_messages, [first.clone()]);
        let facts = recorder.facts.lock().await;
        assert_eq!(facts.errors.len(), 1);
        assert_eq!(facts.errors[0].2, [first.clone()]);
    }
}

#[tokio::test]
async fn stream_deadline_records_completed_progress() {
    let message = completed("finished before deadline", MessagePhase::Commentary);
    let recorder = Arc::new(CollectingExecutionRecorder::default());
    let binding = ModelExecutionBinding::with_recorder(
        ExecutionScope::fresh(CancellationToken::new()),
        recorder.clone(),
    );
    let model = BoundModel::new(
        Arc::new(ModelService::new(Arc::new(CompletedThenPendingAdapter {
            message: message.clone(),
        }))),
        binding,
        30,
    );

    let error = model
        .complete(request("completed-then-pending", "deadline"))
        .await
        .unwrap_err();
    let failure = ModelFailure::from_error(&error);
    assert_eq!(failure.message, "model request timed out after 30ms");
    assert_eq!(failure.completed_messages, [message.clone()]);
    assert_eq!(recorder.facts.lock().await.errors[0].2, [message]);
}

#[tokio::test]
async fn completed_progress_rejects_request_ids_and_tool_parts() {
    let request = request("failure-progress", "invalid");
    let mut reused = completed("bad id", MessagePhase::Commentary);
    reused.id = request.messages[0].id;
    let mut progress = super::super::progress::CompletedMessageProgress::new(&request);
    let error = progress.accept(reused).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("conflicts with the request history")
    );

    let tool_message = CanonicalMessage::from_parts(
        MessageRole::Assistant,
        vec![CanonicalPart::new(
            PartProvenance::Model,
            PartScope::Conversation,
            ContentPart::ToolCall {
                call: ToolCall::new("bad_tool", "read_file", serde_json::json!({})),
            },
        )],
    );
    let error = progress.accept(tool_message).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot contain a tool call or result")
    );
}
