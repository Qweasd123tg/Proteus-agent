use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures_util::StreamExt;

use crate::{
    contracts::{EventEmitter, ExecutionScope, Model, ModelEventStream},
    core::{
        JournalEntry, ModelRequestRecorded, ModelResponseOutcome, ModelResponseRecorded,
        ModelService, SessionStore,
    },
    domain::{
        Event, EventContext, ExchangeId, ModelRef, SessionId, ThreadId, ToolSpec, TurnId,
        new_exchange_id,
    },
    model_standard::{
        CanonicalModelRequest, CanonicalModelResponse, ModelCapabilities, ModelStreamEvent,
        validate_model_response_against_request,
    },
};

const RESERVED_ATTRIBUTION_KEYS: [&str; 3] = ["session_id", "thread_id", "turn_id"];

/// Immutable model attribution for one logical execution.
///
/// A detached binding proves that model execution itself does not require a
/// conversational owner. A turn binding adds the current chat/journal
/// projection without putting those identities into `ExecutionScope`.
#[derive(Clone)]
pub struct ModelExecutionBinding {
    scope: ExecutionScope,
    turn: Option<ModelTurnAttribution>,
}

#[derive(Clone)]
struct ModelTurnAttribution {
    events: Arc<EventEmitter>,
    session_id: SessionId,
    thread_id: ThreadId,
    turn_id: TurnId,
    session_store: Option<SessionStore>,
}

impl ModelExecutionBinding {
    pub fn detached(scope: ExecutionScope) -> Self {
        Self { scope, turn: None }
    }

    pub fn for_turn(
        scope: ExecutionScope,
        events: Arc<EventEmitter>,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        session_store: Option<SessionStore>,
    ) -> Self {
        Self {
            scope,
            turn: Some(ModelTurnAttribution {
                events,
                session_id,
                thread_id,
                turn_id,
                session_store,
            }),
        }
    }

    pub fn scope(&self) -> &ExecutionScope {
        &self.scope
    }

    fn bind_request(&self, request: &mut CanonicalModelRequest) -> Result<()> {
        let Some(turn) = &self.turn else {
            if let Some(key) = RESERVED_ATTRIBUTION_KEYS
                .iter()
                .find(|key| request.client_metadata.contains_key(**key))
            {
                return Err(anyhow!(
                    "detached model execution cannot claim reserved client_metadata.{key}"
                ));
            }
            return Ok(());
        };

        bind_reserved_id(request, "session_id", turn.session_id)?;
        bind_reserved_id(request, "thread_id", turn.thread_id)?;
        bind_reserved_id(request, "turn_id", turn.turn_id)?;
        Ok(())
    }

    fn journal_context(&self) -> Result<Option<ModelJournalContext>> {
        let Some(turn) = &self.turn else {
            return Ok(None);
        };
        let Some(store) = turn.session_store.clone() else {
            return Ok(None);
        };
        if store.session_id() != turn.session_id {
            return Err(anyhow!(
                "model binding session_id {} does not match journal session {}",
                turn.session_id,
                store.session_id()
            ));
        }
        Ok(Some((store, turn.thread_id, turn.turn_id)))
    }

    async fn emit_delta(&self, event: Event) {
        let Some(turn) = &self.turn else {
            return;
        };
        let context = EventContext::new(turn.session_id, turn.thread_id, Some(turn.turn_id));
        // A failed presentation sink must not fail a model call.
        let _ = turn.events.emit(context, event).await;
    }
}

/// A model capability bound immutably to one `ExecutionScope`.
///
/// The provider service may be shared, but attribution and cancellation never
/// live in that shared service.
pub struct BoundModel {
    service: Arc<ModelService>,
    binding: ModelExecutionBinding,
}

impl BoundModel {
    pub fn new(service: Arc<ModelService>, binding: ModelExecutionBinding) -> Self {
        Self { service, binding }
    }

    pub fn binding(&self) -> &ModelExecutionBinding {
        &self.binding
    }
}

#[async_trait]
impl Model for BoundModel {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        self.service.id()
    }

    fn capabilities(&self, model: &ModelRef) -> ModelCapabilities {
        self.service.capabilities(model)
    }

    fn provider_hosted_tools(&self, model: &ModelRef) -> Vec<ToolSpec> {
        self.service.provider_hosted_tools(model)
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        let mut request = self.service.prepare_request(request)?;
        self.binding.bind_request(&mut request)?;
        let journal_context = self.binding.journal_context()?;
        let exchange_id = new_exchange_id();

        if let Some((store, thread_id, turn_id)) = &journal_context {
            store
                .append_journal_entry(
                    *thread_id,
                    Some(*turn_id),
                    JournalEntry::ModelRequestRecorded(ModelRequestRecorded {
                        exchange_id,
                        request: request.clone(),
                    }),
                )
                .await?;
        }

        let validation_request = request.clone();
        let cancellation = self.binding.scope.cancellation.clone();
        let stream = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(model_cancelled()),
            result = self.service.start_prepared(request) => match result {
                Ok(stream) => stream,
                Err(error) => {
                    if let Some((store, thread_id, turn_id)) = &journal_context {
                        record_model_outcome(
                            store,
                            *thread_id,
                            *turn_id,
                            exchange_id,
                            ModelResponseOutcome::Error {
                                message: format!("model adapter error: {error:#}"),
                            },
                        )
                        .await?;
                    }
                    return Err(error);
                }
            },
        };

        Ok(bound_recording_stream(
            stream,
            validation_request,
            journal_context,
            exchange_id,
            cancellation,
        ))
    }

    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        let suppress_stream_deltas = request
            .metadata
            .get("suppress_stream_deltas")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mut stream = self.stream(request).await?;

        while let Some(event) = stream.next().await {
            match event? {
                ModelStreamEvent::Response { response } => return Ok(response),
                ModelStreamEvent::Error { message } => {
                    return Err(anyhow!("model stream error: {message}"));
                }
                ModelStreamEvent::TextDelta { text } if !suppress_stream_deltas => {
                    self.binding
                        .emit_delta(Event::AssistantTextDelta { text })
                        .await;
                }
                ModelStreamEvent::ToolCallDelta {
                    call_id,
                    args_delta,
                    ..
                } if !suppress_stream_deltas => {
                    self.binding
                        .emit_delta(Event::AssistantToolArgsDelta {
                            call_id,
                            args_delta,
                        })
                        .await;
                }
                ModelStreamEvent::ReasoningSummaryDelta { text } if !suppress_stream_deltas => {
                    self.binding
                        .emit_delta(Event::AssistantReasoningDelta { text })
                        .await;
                }
                _ => {}
            }
        }
        Err(anyhow!("model stream ended without Response event"))
    }
}

type ModelJournalContext = (SessionStore, ThreadId, TurnId);

fn bind_reserved_id(
    request: &mut CanonicalModelRequest,
    key: &str,
    bound: uuid::Uuid,
) -> Result<()> {
    if let Some(value) = request.client_metadata.get(key) {
        let requested = value
            .parse::<uuid::Uuid>()
            .map_err(|error| anyhow!("invalid model request client_metadata.{key}: {error}"))?;
        if requested != bound {
            return Err(anyhow!(
                "model request client_metadata.{key} {requested} conflicts with bound value {bound}"
            ));
        }
    } else {
        request
            .client_metadata
            .insert(key.to_owned(), bound.to_string());
    }
    Ok(())
}

fn bound_recording_stream(
    mut stream: ModelEventStream,
    validation_request: CanonicalModelRequest,
    journal_context: Option<ModelJournalContext>,
    exchange_id: ExchangeId,
    cancellation: crate::contracts::CancellationToken,
) -> ModelEventStream {
    Box::pin(async_stream::try_stream! {
        let mut terminal_recorded = false;
        loop {
            let item = tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err(model_cancelled()),
                item = stream.next() => Ok(item),
            }?;
            let Some(item) = item else {
                break;
            };
            let event = match item {
                Ok(event) => event,
                Err(error) => {
                    if let Some((store, thread_id, turn_id)) = &journal_context {
                        record_model_outcome(
                            store,
                            *thread_id,
                            *turn_id,
                            exchange_id,
                            ModelResponseOutcome::Error {
                                message: format!("model stream transport error: {error:#}"),
                            },
                        )
                        .await?;
                    }
                    Err(error)?;
                    unreachable!();
                }
            };
            match &event {
                ModelStreamEvent::Response { response } => {
                    if let Err(error) =
                        validate_model_response_against_request(&validation_request, response)
                    {
                        let error = anyhow!("model protocol error: {error}");
                        if let Some((store, thread_id, turn_id)) = &journal_context {
                            record_model_outcome(
                                store,
                                *thread_id,
                                *turn_id,
                                exchange_id,
                                ModelResponseOutcome::Error {
                                    message: error.to_string(),
                                },
                            )
                            .await?;
                        }
                        Err(error)?;
                    }
                    if let Some((store, thread_id, turn_id)) = &journal_context {
                        record_model_outcome(
                            store,
                            *thread_id,
                            *turn_id,
                            exchange_id,
                            ModelResponseOutcome::Response {
                                response: response.clone(),
                            },
                        )
                        .await?;
                    }
                    terminal_recorded = true;
                    yield event;
                    break;
                }
                ModelStreamEvent::Error { message } => {
                    if let Some((store, thread_id, turn_id)) = &journal_context {
                        record_model_outcome(
                            store,
                            *thread_id,
                            *turn_id,
                            exchange_id,
                            ModelResponseOutcome::Error {
                                message: message.clone(),
                            },
                        )
                        .await?;
                    }
                    terminal_recorded = true;
                    yield event;
                    break;
                }
                _ => yield event,
            }
        }
        if !terminal_recorded {
            let message = "model stream ended without Response event".to_owned();
            if let Some((store, thread_id, turn_id)) = &journal_context {
                record_model_outcome(
                    store,
                    *thread_id,
                    *turn_id,
                    exchange_id,
                    ModelResponseOutcome::Error {
                        message: message.clone(),
                    },
                )
                .await?;
            }
            Err(anyhow!(message))?;
        }
    })
}

async fn record_model_outcome(
    store: &SessionStore,
    thread_id: ThreadId,
    turn_id: TurnId,
    exchange_id: ExchangeId,
    outcome: ModelResponseOutcome,
) -> Result<()> {
    store
        .append_journal_entry(
            thread_id,
            Some(turn_id),
            JournalEntry::ModelResponseRecorded(ModelResponseRecorded {
                exchange_id,
                outcome,
            }),
        )
        .await?;
    Ok(())
}

fn model_cancelled() -> anyhow::Error {
    anyhow!("model execution canceled")
}

#[cfg(test)]
#[path = "bound_model_tests.rs"]
mod tests;
