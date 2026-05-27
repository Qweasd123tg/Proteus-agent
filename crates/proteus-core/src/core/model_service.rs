use std::sync::{Arc, RwLock};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures_util::StreamExt;

use crate::{
    contracts::{EventEmitter, ModelAdapter, ModelClient, ModelEventStream},
    domain::{Event, EventContext, ModelRef, SessionId, ThreadId, TurnId},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, FinishReason, MessageRole,
        ModelCapabilities, ModelStreamEvent, RequestShaper,
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
}

pub struct ModelService {
    adapter: Arc<dyn ModelAdapter>,
    shaper: RequestShaper,
    delta_context: RwLock<DeltaEventContext>,
}

impl ModelService {
    pub fn new(adapter: Arc<dyn ModelAdapter>) -> Self {
        Self {
            adapter,
            shaper: RequestShaper,
            delta_context: RwLock::new(DeltaEventContext::default()),
        }
    }

    pub fn with_shaper(adapter: Arc<dyn ModelAdapter>, shaper: RequestShaper) -> Self {
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
impl ModelClient for ModelService {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        self.adapter.id()
    }

    fn capabilities(&self, model: &ModelRef) -> ModelCapabilities {
        self.adapter.capabilities(model)
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let capabilities = self.adapter.capabilities(&request.model);
        let request = self.shaper.shape(request, &capabilities)?;
        self.adapter.stream(request).await
    }

    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        let ctx = self.snapshot_context();
        let mut stream = self.stream(request).await?;
        let mut text = String::new();
        let mut saw_tool_delta = false;
        let mut saw_tool_finished = false;
        let mut done_reason = None;

        while let Some(event) = stream.next().await {
            let event = event?;
            match event {
                ModelStreamEvent::Response { response } => return Ok(response),
                ModelStreamEvent::Error { message } => {
                    return Err(anyhow!("model stream error: {message}"));
                }
                ModelStreamEvent::TextDelta { text: delta } => {
                    emit_delta(
                        &ctx,
                        Event::AssistantTextDelta {
                            text: delta.clone(),
                        },
                    )
                    .await;
                    text.push_str(&delta);
                }
                ModelStreamEvent::ToolCallDelta {
                    call_id,
                    args_delta,
                    ..
                } => {
                    saw_tool_delta = true;
                    emit_delta(
                        &ctx,
                        Event::AssistantToolArgsDelta {
                            call_id,
                            args_delta,
                        },
                    )
                    .await;
                }
                ModelStreamEvent::ReasoningSummaryDelta { text } => {
                    emit_delta(&ctx, Event::AssistantReasoningDelta { text }).await;
                }
                ModelStreamEvent::ToolCallFinished { .. } => {
                    saw_tool_finished = true;
                }
                ModelStreamEvent::Done { finish_reason } => {
                    done_reason = Some(finish_reason);
                }
                // Usage пока не эмитим как runtime event — в нём нет
                // UI-полезной нагрузки сверх Response.
                _ => {}
            }
        }
        if !text.is_empty() && !saw_tool_delta && !saw_tool_finished {
            let reason = done_reason.unwrap_or(FinishReason::Stop);
            return Ok(CanonicalModelResponse::new(
                CanonicalMessage::text(MessageRole::Assistant, text),
                Vec::new(),
                reason,
            )
            .with_provider_metadata(serde_json::json!({
                "synthesized_from_text_deltas": true
            })));
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
        contracts::{EventSink, ModelAdapter},
        domain::{EventEnvelope, ModelRef, new_session_id, new_thread_id, new_turn_id},
        model_standard::{
            CanonicalMessage, CanonicalModelResponse, FinishReason, MessageRole, ModelStreamEvent,
        },
    };
    use futures_util::stream;
    use tokio::sync::Mutex as AsyncMutex;

    /// Адаптер, отдающий зафиксированный список stream events.
    struct ScriptedAdapter {
        events: std::sync::Mutex<Option<Vec<ModelStreamEvent>>>,
    }

    impl ScriptedAdapter {
        fn new(events: Vec<ModelStreamEvent>) -> Self {
            Self {
                events: std::sync::Mutex::new(Some(events)),
            }
        }
    }

    #[async_trait]
    impl ModelAdapter for ScriptedAdapter {
        fn id(&self) -> std::borrow::Cow<'static, str> {
            "scripted".into()
        }
        fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
            ModelCapabilities::empty()
        }
        async fn stream(&self, _request: CanonicalModelRequest) -> Result<ModelEventStream> {
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
    async fn stream_ending_with_text_without_response_synthesizes_response() {
        let adapter = Arc::new(ScriptedAdapter::new(vec![ModelStreamEvent::TextDelta {
            text: "foo".into(),
        }]));
        let service = ModelService::new(adapter);
        let response = service.complete(sample_request()).await.unwrap();
        assert_eq!(response.finish_reason, FinishReason::Stop);
        assert!(matches!(
            response.message.parts.as_slice(),
            [crate::model_standard::ContentPart::Text { text }] if text == "foo"
        ));
        assert_eq!(
            response.provider_metadata["synthesized_from_text_deltas"],
            true
        );
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
