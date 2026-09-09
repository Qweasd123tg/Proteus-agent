use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures_util::StreamExt;

#[path = "bound_model/presentation.rs"]
mod presentation;
#[path = "bound_model/progress.rs"]
mod progress;

use progress::CompletedMessageProgress;

use crate::{
    contracts::{
        EventEmitter, ExecutionRecorder, ExecutionScope, Model, ModelEventStream,
        NoopExecutionRecorder,
    },
    core::{ModelService, model_call_scope::current_model_call_origin},
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
    recorder: Arc<dyn ExecutionRecorder>,
    turn: Option<ModelTurnAttribution>,
}

#[derive(Clone)]
struct ModelTurnAttribution {
    events: Arc<EventEmitter>,
    session_id: SessionId,
    thread_id: ThreadId,
    turn_id: TurnId,
}

impl ModelExecutionBinding {
    pub fn detached(scope: ExecutionScope) -> Self {
        Self::with_recorder(scope, Arc::new(NoopExecutionRecorder))
    }

    pub fn with_recorder(scope: ExecutionScope, recorder: Arc<dyn ExecutionRecorder>) -> Self {
        Self {
            scope,
            recorder,
            turn: None,
        }
    }

    pub fn for_turn(
        scope: ExecutionScope,
        events: Arc<EventEmitter>,
        session_id: SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
        recorder: Arc<dyn ExecutionRecorder>,
    ) -> Self {
        Self {
            scope,
            recorder,
            turn: Some(ModelTurnAttribution {
                events,
                session_id,
                thread_id,
                turn_id,
            }),
        }
    }

    pub fn scope(&self) -> &ExecutionScope {
        &self.scope
    }

    pub fn recorder(&self) -> Arc<dyn ExecutionRecorder> {
        self.recorder.clone()
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

    async fn emit_delta(&self, event: Event) {
        let Some(turn) = &self.turn else {
            return;
        };
        let context = EventContext::new(turn.session_id, turn.thread_id, Some(turn.turn_id));
        // A failed presentation sink must not fail a model call.
        let _ = turn.events.emit(context, event).await;
    }

    async fn emit_delta_before_deadline(
        &self,
        event: Event,
        deadline: Option<tokio::time::Instant>,
    ) -> bool {
        tokio::select! {
            biased;
            _ = wait_for_deadline(deadline), if deadline.is_some() => false,
            _ = self.emit_delta(event) => true,
        }
    }

    async fn emit_message(
        &self,
        message: &crate::model_standard::CanonicalMessage,
        completed: &mut std::collections::HashMap<
            crate::domain::MessageId,
            (Option<crate::model_standard::MessagePhase>, String),
        >,
        deadline: Option<tokio::time::Instant>,
    ) -> bool {
        let text = message.display_text();
        if !text.is_empty() && completed.get(&message.id) != Some(&(message.phase, text.clone())) {
            completed.insert(message.id, (message.phase, text.clone()));
            return self
                .emit_delta_before_deadline(
                    Event::AssistantMessageCompleted {
                        message_id: message.id,
                        phase: message.phase,
                        text,
                    },
                    deadline,
                )
                .await;
        }
        true
    }
}

/// A model capability bound immutably to one `ExecutionScope`.
///
/// The provider service may be shared, but attribution and cancellation never
/// live in that shared service.
pub struct BoundModel {
    service: Arc<ModelService>,
    binding: ModelExecutionBinding,
    model_timeout_ms: u64,
}

impl BoundModel {
    pub(crate) fn new(
        service: Arc<ModelService>,
        binding: ModelExecutionBinding,
        model_timeout_ms: u64,
    ) -> Self {
        Self {
            service,
            binding,
            model_timeout_ms,
        }
    }

    pub fn binding(&self) -> &ModelExecutionBinding {
        &self.binding
    }

    fn deadline(&self) -> Option<tokio::time::Instant> {
        (self.model_timeout_ms != 0)
            .then(|| tokio::time::Instant::now() + Duration::from_millis(self.model_timeout_ms))
    }

    async fn stream_with_deadline(
        &self,
        request: CanonicalModelRequest,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<ModelEventStream> {
        let mut request = self.service.prepare_request(request)?;
        self.binding.bind_request(&mut request)?;
        let exchange_id = new_exchange_id();

        self.binding
            .recorder
            .model_request_recorded(exchange_id, current_model_call_origin(), &request)
            .await?;

        let validation_request = request.clone();
        let cancellation = self.binding.scope.cancellation.clone();
        let recorder = self.binding.recorder();
        let stream = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(model_cancelled()),
            _ = wait_for_deadline(deadline), if deadline.is_some() => {
                let error = model_timeout(self.model_timeout_ms);
                let mut progress = CompletedMessageProgress::new(&validation_request);
                let failure = progress.attach_to_failure(
                    crate::model_standard::ModelFailure::from_error(&error),
                );
                let failure = record_failure(&recorder, exchange_id, failure).await?;
                return Err(anyhow::Error::new(failure));
            }
            result = self.service.start_prepared(request) => match result {
                Ok(stream) => stream,
                Err(error) => {
                    let mut progress = CompletedMessageProgress::new(&validation_request);
                    let failure = progress.attach_to_failure(
                        crate::model_standard::ModelFailure::from_error(&error),
                    );
                    let failure = record_failure(&recorder, exchange_id, failure).await?;
                    return Err(anyhow::Error::new(failure));
                }
            },
        };

        Ok(bound_recording_stream(
            stream,
            validation_request,
            recorder,
            exchange_id,
            cancellation,
            deadline,
            self.model_timeout_ms,
        ))
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
        let deadline = self.deadline();
        let suppress = request
            .metadata
            .get("suppress_stream_deltas")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let stream = self.stream_with_deadline(request, deadline).await?;
        Ok(presentation::present_stream(
            stream,
            self.binding.clone(),
            suppress,
            deadline,
        ))
    }

    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        let mut stream = self.stream(request).await?;
        while let Some(event) = stream.next().await {
            match event? {
                ModelStreamEvent::Response { response } => return Ok(response),
                ModelStreamEvent::Error { failure } => return Err(failure.into()),
                _ => {}
            }
        }
        Err(anyhow!("model stream ended without Response event"))
    }
}

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
    recorder: Arc<dyn ExecutionRecorder>,
    exchange_id: ExchangeId,
    cancellation: crate::contracts::CancellationToken,
    deadline: Option<tokio::time::Instant>,
    model_timeout_ms: u64,
) -> ModelEventStream {
    Box::pin(async_stream::try_stream! {
        let mut terminal_recorded = false;
        let mut progress = CompletedMessageProgress::new(&validation_request);
        loop {
            enum Next {
                Item(Option<Result<ModelStreamEvent>>),
                Cancelled,
                Deadline,
            }
            let next = tokio::select! {
                biased;
                _ = cancellation.cancelled() => Next::Cancelled,
                _ = wait_for_deadline(deadline), if deadline.is_some() => Next::Deadline,
                item = stream.next() => Next::Item(item),
            };
            let item = match next {
                Next::Item(item) => item,
                Next::Cancelled => Err(model_cancelled())?,
                Next::Deadline => {
                    drop(stream);
                    let error = model_timeout(model_timeout_ms);
                    let failure = progress.attach_to_failure(
                        crate::model_standard::ModelFailure::from_error(&error),
                    );
                    let failure = record_failure(&recorder, exchange_id, failure).await?;
                    Err(anyhow::Error::new(failure))?;
                    unreachable!();
                }
            };
            let Some(item) = item else {
                break;
            };
            let mut event = match item {
                Ok(event) => event,
                Err(error) => {
                    let failure = progress.attach_to_failure(
                        crate::model_standard::ModelFailure::from_error(&error),
                    );
                    let failure = record_failure(&recorder, exchange_id, failure).await?;
                    Err(anyhow::Error::new(failure))?;
                    unreachable!();
                }
            };
            match &mut event {
                ModelStreamEvent::Response { response } => {
                    if let Err(error) =
                        progress.reconcile_response(response).and_then(|_| {
                            validate_model_response_against_request(&validation_request, response)
                                .map_err(anyhow::Error::msg)
                        })
                    {
                        let error = anyhow!("model protocol error: {error}");
                        let failure = progress.attach_to_failure(
                            crate::model_standard::ModelFailure::from_error(&error),
                        );
                        let failure = record_failure(&recorder, exchange_id, failure).await?;
                        Err(anyhow::Error::new(failure))?;
                    }
                    recorder
                        .model_response_recorded(exchange_id, response)
                        .await?;
                    terminal_recorded = true;
                    yield event;
                    break;
                }
                ModelStreamEvent::Error { failure } => {
                    let failure = progress.attach_to_failure(failure.clone());
                    let failure = record_failure(&recorder, exchange_id, failure).await?;
                    terminal_recorded = true;
                    yield ModelStreamEvent::Error { failure };
                    break;
                }
                ModelStreamEvent::MessageCompleted { message } => {
                    let accepted = progress.accept(message.clone());
                    if let Err(error) = &accepted {
                        let failure = progress.attach_to_failure(
                            crate::model_standard::ModelFailure::other(format!(
                                "model protocol error: {error}"
                            )),
                        );
                        let failure = record_failure(&recorder, exchange_id, failure).await?;
                        Err(anyhow::Error::new(failure))?;
                    }
                    if accepted? {
                        recorder.model_stream_event_recorded(exchange_id, &event).await?;
                        yield event;
                    }
                }
                _ => yield event,
            }
        }
        if !terminal_recorded {
            let message = "model stream ended without Response event".to_owned();
            let failure = progress.attach_to_failure(
                crate::model_standard::ModelFailure::other(message),
            );
            let failure = record_failure(&recorder, exchange_id, failure).await?;
            Err(anyhow::Error::new(failure))?;
        }
    })
}

async fn record_failure(
    recorder: &Arc<dyn ExecutionRecorder>,
    exchange_id: ExchangeId,
    failure: crate::model_standard::ModelFailure,
) -> Result<crate::model_standard::ModelFailure> {
    recorder.model_error_recorded(exchange_id, &failure).await?;
    Ok(failure)
}

async fn wait_for_deadline(deadline: Option<tokio::time::Instant>) {
    tokio::time::sleep_until(deadline.expect("deadline branch is disabled without a deadline"))
        .await;
}

fn model_timeout(model_timeout_ms: u64) -> anyhow::Error {
    crate::model_standard::ModelFailure::other(format!(
        "model request timed out after {model_timeout_ms}ms"
    ))
    .into()
}

fn model_cancelled() -> anyhow::Error {
    crate::model_standard::ModelFailure::new(
        crate::model_standard::ModelFailureKind::Interrupted,
        "model execution canceled",
    )
    .into()
}

#[cfg(test)]
#[path = "bound_model_tests.rs"]
mod tests;
