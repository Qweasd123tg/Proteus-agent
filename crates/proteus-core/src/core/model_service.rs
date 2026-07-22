use std::sync::{Arc, RwLock};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures_util::StreamExt;

use crate::{
    contracts::{EventEmitter, Model, ModelEventStream},
    core::RequestSnapshotWriter,
    domain::{Event, EventContext, ModelRef, SessionId, ThreadId, TurnId},
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, ModelCapabilities, ModelStreamEvent,
        RequestShaper, validate_model_response_against_request,
    },
};

/// Источник контекста для эмиссии delta-событий из ModelService.
///
/// Хранится под `RwLock<Option<...>>` потому что runtime-а (а значит и
/// emitter'а) на момент создания ModelService ещё нет. BuiltinRegistry
/// строится ДО runtime-контекста; выставляется перед вызовом
/// `complete()` через `set_event_context`.
#[derive(Clone, Default)]
pub struct DeltaEventContext {
    pub emitter: Option<Arc<EventEmitter>>,
    pub session_id: Option<SessionId>,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub request_snapshot_writer: Option<Arc<RequestSnapshotWriter>>,
}

pub struct ModelService {
    adapter: Arc<dyn Model>,
    shaper: RequestShaper,
    delta_context: RwLock<DeltaEventContext>,
}

impl ModelService {
    pub fn new(adapter: Arc<dyn Model>) -> Self {
        Self {
            adapter,
            shaper: RequestShaper,
            delta_context: RwLock::new(DeltaEventContext::default()),
        }
    }

    pub fn with_shaper(adapter: Arc<dyn Model>, shaper: RequestShaper) -> Self {
        Self {
            adapter,
            shaper,
            delta_context: RwLock::new(DeltaEventContext::default()),
        }
    }

    /// Вставляет emitter + текущий session/thread/turn, чтобы delta-события
    /// прилетали в event log с правильным envelope context. Вызывается из
    /// runtime перед каждым turn'ом (или однократно при создании, если
    /// context не меняется).
    pub fn set_event_context(&self, ctx: DeltaEventContext) {
        if let Ok(mut guard) = self.delta_context.write() {
            *guard = ctx;
        }
    }

    fn snapshot_context(&self) -> DeltaEventContext {
        self.delta_context
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
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

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let capabilities = self.adapter.capabilities(&request.model);
        let ctx = self.snapshot_context();
        let mut request = self.shaper.shape(request, &capabilities)?;
        if let Some(session_id) = ctx.session_id {
            request
                .client_metadata
                .entry("session_id".to_owned())
                .or_insert_with(|| session_id.to_string());
        }
        if let Some(thread_id) = ctx.thread_id {
            request
                .client_metadata
                .entry("thread_id".to_owned())
                .or_insert_with(|| thread_id.to_string());
        }
        if let Some(turn_id) = ctx.turn_id {
            request
                .client_metadata
                .entry("turn_id".to_owned())
                .or_insert_with(|| turn_id.to_string());
        }
        if let (Some(writer), Some(thread_id)) = (&ctx.request_snapshot_writer, ctx.thread_id)
            && let Err(error) = writer.append(thread_id, &request).await
        {
            eprintln!("warning: failed to persist model request snapshot: {error:#}");
        }
        let validation_request = request.clone();
        let stream = self.adapter.stream(request).await?;
        Ok(Box::pin(stream.map(move |event| {
            let event = event?;
            if let ModelStreamEvent::Response { response } = &event {
                validate_model_response_against_request(&validation_request, response)
                    .map_err(|error| anyhow!("model protocol error: {error}"))?;
            }
            Ok(event)
        })))
    }

    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        let suppress_stream_deltas = request
            .metadata
            .get("suppress_stream_deltas")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let ctx = self.snapshot_context();
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
                ModelStreamEvent::TextDelta { text: delta } => {
                    if !suppress_stream_deltas {
                        emit_delta(
                            &ctx,
                            Event::AssistantTextDelta {
                                text: delta.clone(),
                            },
                        )
                        .await;
                    }
                }
                ModelStreamEvent::ToolCallDelta {
                    call_id,
                    args_delta,
                    ..
                } => {
                    if !suppress_stream_deltas {
                        emit_delta(
                            &ctx,
                            Event::AssistantToolArgsDelta {
                                call_id,
                                args_delta,
                            },
                        )
                        .await;
                    }
                }
                ModelStreamEvent::ReasoningSummaryDelta { text } if !suppress_stream_deltas => {
                    emit_delta(&ctx, Event::AssistantReasoningDelta { text }).await;
                }
                ModelStreamEvent::ReasoningSummaryDelta { .. } => {}
                // Usage пока не эмитим как runtime event — в нём нет
                // UI-полезной нагрузки сверх Response.
                _ => {}
            }
        }
        Err(anyhow!("model stream ended without Response event"))
    }
}

async fn emit_delta(ctx: &DeltaEventContext, event: Event) {
    let (Some(emitter), Some(session_id), Some(thread_id)) =
        (&ctx.emitter, ctx.session_id, ctx.thread_id)
    else {
        // Без полного envelope context дельты просто дропаем — это
        // штатное поведение в тестах и для режима без runtime.
        return;
    };
    let envelope_ctx = EventContext::new(session_id, thread_id, ctx.turn_id);
    // Ошибки эмиссии намеренно игнорируем: сломавшийся sink не должен
    // валить model call.
    let _ = emitter.emit(envelope_ctx, event).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        contracts::{EventSink, Model},
        domain::{
            EventEnvelope, ModelRef, ToolCall, ToolCallSurface, ToolSafety, ToolSpec, ToolSurface,
            new_call_id, new_session_id, new_thread_id, new_turn_id,
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
        let service = ModelService::new(adapter);
        let sink = Arc::new(CollectingSink::default());
        let emitter = Arc::new(EventEmitter::new(sink.clone()));
        service.set_event_context(DeltaEventContext {
            emitter: Some(emitter),
            session_id: Some(new_session_id()),
            thread_id: Some(new_thread_id()),
            turn_id: Some(new_turn_id()),
            ..DeltaEventContext::default()
        });

        let _ = service.complete(sample_request()).await.unwrap();
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
    async fn model_service_adds_trace_ids_to_client_metadata() {
        let adapter = Arc::new(ScriptedAdapter::new(vec![ModelStreamEvent::Response {
            response: final_response(),
        }]));
        let service = ModelService::new(adapter.clone());
        let session_id = new_session_id();
        let thread_id = new_thread_id();
        let turn_id = new_turn_id();
        service.set_event_context(DeltaEventContext {
            session_id: Some(session_id),
            thread_id: Some(thread_id),
            turn_id: Some(turn_id),
            ..DeltaEventContext::default()
        });

        service.complete(sample_request()).await.unwrap();

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
        let service = ModelService::new(adapter);
        let sink = Arc::new(CollectingSink::default());
        let emitter = Arc::new(EventEmitter::new(sink.clone()));
        service.set_event_context(DeltaEventContext {
            emitter: Some(emitter),
            session_id: Some(new_session_id()),
            thread_id: Some(new_thread_id()),
            turn_id: Some(new_turn_id()),
            ..DeltaEventContext::default()
        });

        let request = sample_request().with_metadata(serde_json::json!({
            "suppress_stream_deltas": true,
        }));
        let response = service.complete(request).await.unwrap();

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
        let service = ModelService::new(adapter);
        // Нет set_event_context — дельты должны просто потеряться без паники.
        let response = service.complete(sample_request()).await.unwrap();
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
