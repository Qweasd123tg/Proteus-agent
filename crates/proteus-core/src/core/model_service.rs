use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures_util::StreamExt;

use crate::{
    contracts::{Model, ModelEventStream},
    domain::{ModelRef, ToolSpec},
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, ModelCapabilities, ModelStreamEvent,
        RequestShaper, validate_model_response_against_request,
    },
};

/// Shared model provider plus canonical request shaping.
///
/// Execution identity, cancellation and event/journal attribution deliberately
/// live in `BoundModel`, so this service is safe to share between executions.
pub struct ModelService {
    adapter: Arc<dyn Model>,
    shaper: RequestShaper,
}

impl ModelService {
    pub fn new(adapter: Arc<dyn Model>) -> Self {
        Self {
            adapter,
            shaper: RequestShaper,
        }
    }

    pub(crate) fn prepare_request(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<CanonicalModelRequest> {
        let capabilities = self.adapter.capabilities(&request.model);
        self.shaper.shape(request, &capabilities)
    }

    pub(crate) async fn start_prepared(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<ModelEventStream> {
        self.adapter.stream(request).await
    }
}

#[async_trait]
impl Model for ModelService {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        self.adapter.id()
    }

    fn capabilities(&self, model: &ModelRef) -> ModelCapabilities {
        self.adapter.capabilities(model)
    }

    fn provider_hosted_tools(&self, model: &ModelRef) -> Vec<ToolSpec> {
        self.adapter.provider_hosted_tools(model)
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let request = self.prepare_request(request)?;
        let validation_request = request.clone();
        let stream = self.start_prepared(request).await?;
        Ok(validating_stream(stream, validation_request))
    }

    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        let mut stream = self.stream(request).await?;

        while let Some(event) = stream.next().await {
            let event = event?;
            match event {
                ModelStreamEvent::Response { response } => {
                    return Ok(response);
                }
                ModelStreamEvent::Error { message } => {
                    return Err(anyhow!("model stream error: {message}"));
                }
                _ => {}
            }
        }
        Err(anyhow!("model stream ended without Response event"))
    }
}

fn validating_stream(
    mut stream: ModelEventStream,
    validation_request: CanonicalModelRequest,
) -> ModelEventStream {
    Box::pin(async_stream::try_stream! {
        let mut terminal_seen = false;
        while let Some(item) = stream.next().await {
            let event = item?;
            match &event {
                ModelStreamEvent::Response { response } => {
                    validate_model_response_against_request(&validation_request, response)
                        .map_err(|error| anyhow!("model protocol error: {error}"))?;
                    terminal_seen = true;
                    yield event;
                    break;
                }
                ModelStreamEvent::Error { .. } => {
                    terminal_seen = true;
                    yield event;
                    break;
                }
                _ => yield event,
            }
        }
        if !terminal_seen {
            Err(anyhow!("model stream ended without Response event"))?;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        contracts::{
            CancellationToken, EventEmitter, EventSink, ExecutionScope, Model,
            NoopExecutionRecorder,
        },
        core::{BoundModel, ModelExecutionBinding},
        domain::{
            Event, EventEnvelope, ModelRef, ToolCall, ToolCallSurface, ToolSafety, ToolSpec,
            ToolSurface, new_call_id, new_session_id, new_thread_id, new_turn_id,
        },
        model_standard::{
            CanonicalMessage, CanonicalModelResponse, ContentPart, FinishReason, MessageRole,
            ModelStreamEvent,
        },
    };
    use futures_util::stream;
    use tokio::sync::Mutex as AsyncMutex;

    /// Адаптер, отдающий зафиксированный список stream events.
    struct ScriptedAdapter {
        events: std::sync::Mutex<Option<Vec<ModelStreamEvent>>>,
        requests: std::sync::Mutex<Vec<CanonicalModelRequest>>,
        capabilities: ModelCapabilities,
    }

    impl ScriptedAdapter {
        fn new(events: Vec<ModelStreamEvent>) -> Self {
            Self {
                events: std::sync::Mutex::new(Some(events)),
                requests: std::sync::Mutex::new(Vec::new()),
                capabilities: ModelCapabilities::empty(),
            }
        }

        fn with_capabilities(mut self, capabilities: ModelCapabilities) -> Self {
            self.capabilities = capabilities;
            self
        }
    }

    #[async_trait]
    impl Model for ScriptedAdapter {
        fn id(&self) -> std::borrow::Cow<'static, str> {
            "scripted".into()
        }
        fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
            self.capabilities.clone()
        }
        async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
            self.requests.lock().unwrap().push(request);
            let events = self
                .events
                .lock()
                .unwrap()
                .take()
                .unwrap_or_default()
                .into_iter()
                .map(Ok)
                .collect::<Vec<_>>();
            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[derive(Default)]
    struct CollectingSink {
        events: AsyncMutex<Vec<EventEnvelope>>,
    }

    #[async_trait]
    impl EventSink for CollectingSink {
        async fn append(&self, envelope: EventEnvelope) -> Result<()> {
            self.events.lock().await.push(envelope);
            Ok(())
        }
    }

    fn final_response() -> CanonicalModelResponse {
        CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "hello"),
            Vec::new(),
            FinishReason::Stop,
        )
    }

    fn sample_request() -> CanonicalModelRequest {
        CanonicalModelRequest::new(
            ModelRef::new("scripted", "x"),
            vec![CanonicalMessage::text(MessageRole::User, "hi")],
        )
    }

    #[tokio::test]
    async fn response_tool_surface_must_match_the_shaped_request() {
        let call = ToolCall::new(new_call_id(), "apply_patch", serde_json::json!(""))
            .with_surface(ToolCallSurface::Function)
            .with_raw_arguments("");
        let response = CanonicalModelResponse::new(
            CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call: call.clone() }],
            ),
            vec![call],
            FinishReason::ToolCalls,
        );
        let adapter = Arc::new(
            ScriptedAdapter::new(vec![ModelStreamEvent::Response { response }]).with_capabilities(
                ModelCapabilities::empty()
                    .with_tools(true)
                    .with_freeform_tools(true),
            ),
        );
        let service = ModelService::new(adapter);
        let request = sample_request().with_tools(vec![
            ToolSpec::new(
                "apply_patch",
                "Apply a patch",
                serde_json::json!({}),
                ToolSafety::WritesFiles,
            )
            .with_surface(ToolSurface::freeform_lark("start: \"*** Begin Patch\"")),
        ]);

        let error = service
            .complete(request)
            .await
            .expect_err("surface mismatch must fail before tool execution");
        assert!(
            error.to_string().contains("model protocol error"),
            "{error}"
        );
        assert!(
            error
                .to_string()
                .contains("used function surface, but request declared freeform surface"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn complete_returns_response_drained_from_stream() {
        let adapter = Arc::new(ScriptedAdapter::new(vec![
            ModelStreamEvent::TextDelta { text: "he".into() },
            ModelStreamEvent::TextDelta { text: "llo".into() },
            ModelStreamEvent::Response {
                response: final_response(),
            },
        ]));
        let service = ModelService::new(adapter);
        let response = service.complete(sample_request()).await.unwrap();
        assert_eq!(response.finish_reason, FinishReason::Stop);
    }

    #[tokio::test]
    async fn empty_final_response_is_not_rewritten_by_model_service() {
        let empty = CanonicalModelResponse::new(
            CanonicalMessage::new(MessageRole::Assistant, Vec::new()),
            Vec::new(),
            FinishReason::Stop,
        )
        .with_end_turn(false);
        let adapter = Arc::new(ScriptedAdapter::new(vec![
            ModelStreamEvent::TextDelta {
                text: "the time ".into(),
            },
            ModelStreamEvent::TextDelta {
                text: "is 12:00".into(),
            },
            ModelStreamEvent::Response { response: empty },
        ]));
        let service = ModelService::new(adapter);
        let response = service.complete(sample_request()).await.unwrap();
        assert!(response.message.parts.is_empty());
        assert_eq!(response.finish_reason, FinishReason::Stop);
        assert_eq!(response.end_turn, Some(false));
    }

    #[tokio::test]
    async fn deltas_flow_to_emitter_when_context_set() {
        let adapter = Arc::new(ScriptedAdapter::new(vec![
            ModelStreamEvent::TextDelta { text: "foo".into() },
            ModelStreamEvent::ToolCallDelta {
                call_id: "call-1".into(),
                name: None,
                args_delta: "{\"a".into(),
            },
            ModelStreamEvent::ReasoningSummaryDelta {
                text: "thinking".into(),
            },
            ModelStreamEvent::Response {
                response: final_response(),
            },
        ]));
        let service = Arc::new(ModelService::new(adapter));
        let sink = Arc::new(CollectingSink::default());
        let emitter = Arc::new(EventEmitter::new(sink.clone()));
        let model = BoundModel::new(
            service,
            ModelExecutionBinding::for_turn(
                ExecutionScope::fresh(CancellationToken::new()),
                emitter,
                new_session_id(),
                new_thread_id(),
                new_turn_id(),
                Arc::new(NoopExecutionRecorder),
            ),
        );

        let _ = model.complete(sample_request()).await.unwrap();
        let captured = sink.events.lock().await;
        let kinds: Vec<&str> = captured
            .iter()
            .map(|e| match &e.event {
                Event::AssistantTextDelta { .. } => "text",
                Event::AssistantToolArgsDelta { .. } => "tool",
                Event::AssistantReasoningDelta { .. } => "reasoning",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["text", "tool", "reasoning"]);
    }

    #[tokio::test]
    async fn bound_model_adds_trace_ids_to_client_metadata() {
        let adapter = Arc::new(ScriptedAdapter::new(vec![ModelStreamEvent::Response {
            response: final_response(),
        }]));
        let service = Arc::new(ModelService::new(adapter.clone()));
        let session_id = new_session_id();
        let thread_id = new_thread_id();
        let turn_id = new_turn_id();
        let model = BoundModel::new(
            service,
            ModelExecutionBinding::for_turn(
                ExecutionScope::fresh(CancellationToken::new()),
                Arc::new(EventEmitter::new(Arc::new(CollectingSink::default()))),
                session_id,
                thread_id,
                turn_id,
                Arc::new(NoopExecutionRecorder),
            ),
        );

        model.complete(sample_request()).await.unwrap();

        let requests = adapter.requests.lock().unwrap();
        assert_eq!(
            requests[0].client_metadata["session_id"],
            session_id.to_string()
        );
        assert_eq!(
            requests[0].client_metadata["thread_id"],
            thread_id.to_string()
        );
        assert_eq!(requests[0].client_metadata["turn_id"], turn_id.to_string());
    }

    #[tokio::test]
    async fn request_metadata_can_suppress_stream_deltas() {
        let adapter = Arc::new(ScriptedAdapter::new(vec![
            ModelStreamEvent::TextDelta { text: "foo".into() },
            ModelStreamEvent::ReasoningSummaryDelta {
                text: "thinking".into(),
            },
            ModelStreamEvent::Response {
                response: final_response(),
            },
        ]));
        let service = Arc::new(ModelService::new(adapter));
        let sink = Arc::new(CollectingSink::default());
        let emitter = Arc::new(EventEmitter::new(sink.clone()));
        let model = BoundModel::new(
            service,
            ModelExecutionBinding::for_turn(
                ExecutionScope::fresh(CancellationToken::new()),
                emitter,
                new_session_id(),
                new_thread_id(),
                new_turn_id(),
                Arc::new(NoopExecutionRecorder),
            ),
        );

        let request = sample_request().with_metadata(serde_json::json!({
            "suppress_stream_deltas": true,
        }));
        let response = model.complete(request).await.unwrap();

        assert_eq!(response.finish_reason, FinishReason::Stop);
        assert!(sink.events.lock().await.is_empty());
    }

    #[tokio::test]
    async fn deltas_dropped_silently_without_emitter() {
        let adapter = Arc::new(ScriptedAdapter::new(vec![
            ModelStreamEvent::TextDelta { text: "hi".into() },
            ModelStreamEvent::Response {
                response: final_response(),
            },
        ]));
        let service = Arc::new(ModelService::new(adapter));
        let model = BoundModel::new(
            service,
            ModelExecutionBinding::detached(ExecutionScope::fresh(CancellationToken::new())),
        );
        let response = model.complete(sample_request()).await.unwrap();
        assert_eq!(response.finish_reason, FinishReason::Stop);
    }

    #[tokio::test]
    async fn stream_error_propagates_as_anyhow() {
        let adapter = Arc::new(ScriptedAdapter::new(vec![ModelStreamEvent::Error {
            message: "provider exploded".into(),
        }]));
        let service = ModelService::new(adapter);
        let err = service.complete(sample_request()).await.unwrap_err();
        assert!(err.to_string().contains("provider exploded"), "{err}");
    }

    #[tokio::test]
    async fn stream_ending_with_text_without_response_is_error() {
        let adapter = Arc::new(ScriptedAdapter::new(vec![ModelStreamEvent::TextDelta {
            text: "foo".into(),
        }]));
        let service = ModelService::new(adapter);
        let err = service.complete(sample_request()).await.unwrap_err();
        assert!(err.to_string().contains("without Response"), "{err}");
    }

    #[tokio::test]
    async fn stream_ending_without_text_or_response_is_error() {
        let adapter = Arc::new(ScriptedAdapter::new(Vec::new()));
        let service = ModelService::new(adapter);
        let err = service.complete(sample_request()).await.unwrap_err();
        assert!(err.to_string().contains("without Response"), "{err}");
    }

    #[tokio::test]
    async fn stream_ending_with_tool_delta_without_response_is_error() {
        let adapter = Arc::new(ScriptedAdapter::new(vec![
            ModelStreamEvent::TextDelta {
                text: "calling".into(),
            },
            ModelStreamEvent::ToolCallDelta {
                call_id: "call-1".into(),
                name: Some("read_file".into()),
                args_delta: "{}".into(),
            },
        ]));
        let service = ModelService::new(adapter);
        let err = service.complete(sample_request()).await.unwrap_err();
        assert!(err.to_string().contains("without Response"), "{err}");
    }
}
