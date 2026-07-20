//! Session-owned root steering queue and model-boundary delivery.
//!
//! The queue belongs to runtime/session, while workflows continue to receive
//! the ordinary `Model` contract. `SteeringModel` observes canonical model
//! responses: a response with tool calls opens one delivery boundary before
//! the next model call. Messages that never reach such a boundary are promoted
//! to follow-up turns after settlement.

use std::{
    collections::{HashSet, VecDeque},
    future::Future,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use anyhow::{Result, anyhow, ensure};
use async_trait::async_trait;
use futures_util::StreamExt;
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::{
    contracts::{EventEmitter, Model, ModelEventStream, WorkflowOutput},
    domain::{
        Event, EventContext, MessageId, ModelRef, SteeringDeliveryKind, ThreadId, TurnId,
        new_turn_id,
    },
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, MessageRole,
        ModelCapabilities, ModelStreamEvent,
    },
};

const MAX_QUEUED_MESSAGES: usize = 32;
const MAX_QUEUED_BYTES: usize = 512 * 1024;
const MAX_MESSAGE_BYTES: usize = 256 * 1024;

tokio::task_local! {
    static ROOT_STEERING_SUPPRESSED: ();
}

/// Runs an internal model call without consuming or observing a root-steering
/// boundary. Compaction uses this scope so queued user input reaches the next
/// workflow model request, never the summarizer request hidden behind the
/// `HistoryCompactor` contract.
pub(crate) async fn without_root_steering<F>(future: F) -> F::Output
where
    F: Future,
{
    ROOT_STEERING_SUPPRESSED.scope((), future).await
}

fn root_steering_is_suppressed() -> bool {
    ROOT_STEERING_SUPPRESSED.try_with(|()| ()).is_ok()
}

#[derive(Debug, Clone)]
pub(crate) struct ReservedUserMessage {
    pub(crate) turn_id: TurnId,
    pub(crate) message: CanonicalMessage,
    pub(crate) text: String,
    pub(crate) delivery: Option<SteeringDeliveryKind>,
}

#[derive(Debug, Clone)]
pub(crate) struct SteeringQueueReceipt {
    pub(crate) message_id: MessageId,
    pub(crate) text: String,
    pub(crate) active_turn_id: TurnId,
    pub(crate) queued_count: usize,
}

#[derive(Debug)]
pub(crate) enum UserMessageReservation {
    Start(ReservedUserMessage),
    Queued(SteeringQueueReceipt),
}

#[derive(Debug, Clone)]
struct QueuedUserMessage {
    message: CanonicalMessage,
    text: String,
}

#[derive(Default)]
struct SteeringQueueState {
    active_turn_id: Option<TurnId>,
    queued: VecDeque<QueuedUserMessage>,
    queued_bytes: usize,
}

/// Bounded queue shared by every root turn in one runtime session.
pub(crate) struct SessionSteering {
    state: StdMutex<SteeringQueueState>,
    queued_count: Arc<AtomicUsize>,
    finalization_gate: Arc<Mutex<()>>,
}

impl Default for SessionSteering {
    fn default() -> Self {
        Self {
            state: StdMutex::new(SteeringQueueState::default()),
            queued_count: Arc::new(AtomicUsize::new(0)),
            finalization_gate: Arc::new(Mutex::new(())),
        }
    }
}

impl SessionSteering {
    pub(crate) async fn reserve(&self, text: String) -> Result<UserMessageReservation> {
        validate_message(&text)?;
        let message = CanonicalMessage::text(MessageRole::User, text.clone());
        let _finalization_guard = self.finalization_gate.lock().await;
        let mut state = self.state.lock().expect("steering state lock");
        let Some(active_turn_id) = state.active_turn_id else {
            let turn_id = new_turn_id();
            state.active_turn_id = Some(turn_id);
            return Ok(UserMessageReservation::Start(ReservedUserMessage {
                turn_id,
                message,
                text,
                delivery: None,
            }));
        };

        ensure!(
            state.queued.len() < MAX_QUEUED_MESSAGES,
            "root steering queue is full (max {MAX_QUEUED_MESSAGES} messages)"
        );
        ensure!(
            state.queued_bytes.saturating_add(text.len()) <= MAX_QUEUED_BYTES,
            "root steering queue byte budget exceeded (max {MAX_QUEUED_BYTES} bytes)"
        );
        let message_id = message.id;
        state.queued_bytes += text.len();
        state.queued.push_back(QueuedUserMessage {
            message,
            text: text.clone(),
        });
        let queued_count = state.queued.len();
        self.queued_count.store(queued_count, Ordering::Release);
        Ok(UserMessageReservation::Queued(SteeringQueueReceipt {
            message_id,
            text,
            active_turn_id,
            queued_count,
        }))
    }

    pub(crate) async fn validate_reservation(&self, turn_id: TurnId) -> Result<()> {
        let state = self.state.lock().expect("steering state lock");
        ensure!(
            state.active_turn_id == Some(turn_id),
            "root turn reservation is no longer active"
        );
        Ok(())
    }

    async fn take_for_steering(&self, turn_id: TurnId) -> Result<Option<QueuedUserMessage>> {
        let mut state = self.state.lock().expect("steering state lock");
        ensure!(
            state.active_turn_id == Some(turn_id),
            "steering delivery targeted a stale root turn"
        );
        Ok(pop_front(&mut state, &self.queued_count))
    }

    pub(crate) async fn settle_and_take_followup(
        self: &Arc<Self>,
        turn_id: TurnId,
    ) -> Result<RootTurnSettlement> {
        let finalization_guard = self.finalization_gate.clone().lock_owned().await;
        let mut state = self.state.lock().expect("steering state lock");
        ensure!(
            state.active_turn_id == Some(turn_id),
            "root turn settlement targeted a stale reservation"
        );
        let Some(queued) = pop_front(&mut state, &self.queued_count) else {
            drop(state);
            return Ok(RootTurnSettlement::Complete(SteeringFinalizationGuard {
                queue: self.clone(),
                _gate: finalization_guard,
            }));
        };
        let next_turn_id = new_turn_id();
        state.active_turn_id = Some(next_turn_id);
        drop(state);
        drop(finalization_guard);
        Ok(RootTurnSettlement::FollowUp(ReservedUserMessage {
            turn_id: next_turn_id,
            message: queued.message,
            text: queued.text,
            delivery: Some(SteeringDeliveryKind::FollowUp),
        }))
    }

    pub(crate) async fn abort(&self) {
        let _finalization_guard = self.finalization_gate.lock().await;
        self.abort_now();
    }

    fn abort_now(&self) {
        let mut state = self.state.lock().expect("steering state lock");
        state.active_turn_id = None;
        state.queued.clear();
        state.queued_bytes = 0;
        self.queued_count.store(0, Ordering::Release);
    }

    pub(crate) async fn queued_messages(&self) -> Vec<(MessageId, String)> {
        self.state
            .lock()
            .expect("steering state lock")
            .queued
            .iter()
            .map(|queued| (queued.message.id, queued.text.clone()))
            .collect()
    }

    pub(crate) fn queued_count_handle(&self) -> Arc<AtomicUsize> {
        self.queued_count.clone()
    }

    pub(crate) fn run_guard(self: &Arc<Self>) -> SteeringRunGuard {
        SteeringRunGuard {
            queue: self.clone(),
            armed: true,
        }
    }

    pub(crate) async fn finalization_guard(self: &Arc<Self>) -> SteeringFinalizationGuard {
        SteeringFinalizationGuard {
            queue: self.clone(),
            _gate: self.finalization_gate.clone().lock_owned().await,
        }
    }
}

pub(crate) enum RootTurnSettlement {
    FollowUp(ReservedUserMessage),
    Complete(SteeringFinalizationGuard),
}

/// Keeps the session reserved until the transport has published the terminal
/// app event. New sends wait at the gate, preventing a stale `TurnOutput` or
/// `Error` from racing with the next root turn.
pub(crate) struct SteeringFinalizationGuard {
    queue: Arc<SessionSteering>,
    _gate: OwnedMutexGuard<()>,
}

impl Drop for SteeringFinalizationGuard {
    fn drop(&mut self) {
        self.queue.abort_now();
    }
}

/// Makes reservation cleanup cancellation-safe even when a transport has to
/// abort the task and Rust drops the runtime future before it can return an
/// error normally.
pub(crate) struct SteeringRunGuard {
    queue: Arc<SessionSteering>,
    armed: bool,
}

impl SteeringRunGuard {
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SteeringRunGuard {
    fn drop(&mut self) {
        if self.armed {
            self.queue.abort_now();
        }
    }
}

fn validate_message(text: &str) -> Result<()> {
    ensure!(!text.trim().is_empty(), "user message must not be empty");
    ensure!(
        text.len() <= MAX_MESSAGE_BYTES,
        "user message exceeds {MAX_MESSAGE_BYTES} byte limit"
    );
    Ok(())
}

fn pop_front(
    state: &mut SteeringQueueState,
    queued_count: &AtomicUsize,
) -> Option<QueuedUserMessage> {
    let queued = state.queued.pop_front()?;
    state.queued_bytes = state.queued_bytes.saturating_sub(queued.text.len());
    queued_count.store(state.queued.len(), Ordering::Release);
    Some(queued)
}

#[derive(Debug, Clone)]
pub(crate) struct SteeringDeliveryRecord {
    pub(crate) message: CanonicalMessage,
    before_message_id: Option<MessageId>,
    awaiting_anchor: bool,
}

/// Per-turn model decorator. It keeps runtime-injected user messages visible
/// in later requests even though the workflow's private history predates them.
#[derive(Clone)]
pub(crate) struct SteeringModel {
    inner: Arc<dyn Model>,
    queue: Arc<SessionSteering>,
    events: Arc<EventEmitter>,
    session_id: crate::domain::SessionId,
    thread_id: ThreadId,
    turn_id: TurnId,
    ready_after_tool_response: Arc<AtomicBool>,
    deliveries: Arc<Mutex<Vec<SteeringDeliveryRecord>>>,
}

impl SteeringModel {
    pub(crate) fn new(
        inner: Arc<dyn Model>,
        queue: Arc<SessionSteering>,
        events: Arc<EventEmitter>,
        session_id: crate::domain::SessionId,
        thread_id: ThreadId,
        turn_id: TurnId,
    ) -> Self {
        Self {
            inner,
            queue,
            events,
            session_id,
            thread_id,
            turn_id,
            ready_after_tool_response: Arc::new(AtomicBool::new(false)),
            deliveries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) async fn delivery_records(&self) -> Vec<SteeringDeliveryRecord> {
        self.deliveries.lock().await.clone()
    }

    async fn prepare_request(
        &self,
        mut request: CanonicalModelRequest,
    ) -> Result<CanonicalModelRequest> {
        {
            let mut deliveries = self.deliveries.lock().await;
            weave_deliveries_into_request(&mut request.messages, &mut deliveries);
        }

        if self.ready_after_tool_response.swap(false, Ordering::AcqRel)
            && let Some(queued) = self.queue.take_for_steering(self.turn_id).await?
        {
            request.messages.push(queued.message.clone());
            self.deliveries.lock().await.push(SteeringDeliveryRecord {
                message: queued.message.clone(),
                before_message_id: None,
                awaiting_anchor: true,
            });
            self.events
                .emit(
                    EventContext::new(self.session_id, self.thread_id, Some(self.turn_id)),
                    Event::SteeringDelivered {
                        message_id: queued.message.id,
                        text: queued.text,
                        kind: SteeringDeliveryKind::Steering,
                        queued_count: self.queue.queued_count.load(Ordering::Acquire),
                    },
                )
                .await?;
        }
        Ok(request)
    }

    async fn observe_response(&self, response: &CanonicalModelResponse) {
        let mut deliveries = self.deliveries.lock().await;
        for delivery in deliveries
            .iter_mut()
            .filter(|delivery| delivery.awaiting_anchor)
        {
            delivery.before_message_id = Some(response.message.id);
            delivery.awaiting_anchor = false;
        }
        self.ready_after_tool_response
            .store(!response.tool_calls.is_empty(), Ordering::Release);
    }
}

#[async_trait]
impl Model for SteeringModel {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        self.inner.id()
    }

    fn capabilities(&self, model: &ModelRef) -> ModelCapabilities {
        self.inner.capabilities(model)
    }

    async fn stream(&self, request: CanonicalModelRequest) -> Result<ModelEventStream> {
        if root_steering_is_suppressed() {
            return self.inner.stream(request).await;
        }
        let request = self.prepare_request(request).await?;
        let stream = self.inner.stream(request).await?;
        let steering = self.clone();
        Ok(Box::pin(stream.then(move |item| {
            let steering = steering.clone();
            async move {
                if let Ok(ModelStreamEvent::Response { response }) = &item {
                    steering.observe_response(response).await;
                }
                item
            }
        })))
    }

    async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
        if root_steering_is_suppressed() {
            return self.inner.complete(request).await;
        }
        let request = self.prepare_request(request).await?;
        let response = self.inner.complete(request).await?;
        self.observe_response(&response).await;
        Ok(response)
    }
}

fn weave_deliveries_into_request(
    messages: &mut Vec<CanonicalMessage>,
    deliveries: &mut [SteeringDeliveryRecord],
) {
    for delivery in deliveries {
        if messages
            .iter()
            .any(|message| message.id == delivery.message.id)
        {
            continue;
        }
        if let Some(index) = delivery
            .before_message_id
            .and_then(|target| messages.iter().position(|message| message.id == target))
        {
            messages.insert(index, delivery.message.clone());
        } else {
            // A workflow-side compaction may have removed the original anchor
            // without ever seeing the injected message. Keep the fresh user
            // instruction and re-anchor it to the next response.
            messages.push(delivery.message.clone());
            delivery.awaiting_anchor = true;
        }
    }
}

/// Weaves core-injected messages into the persistent workflow delta. A
/// workflow is not allowed to manufacture user messages itself; the returned
/// set is passed to history validation as the exact runtime authorization.
pub(crate) fn weave_deliveries_into_output(
    output: &mut WorkflowOutput,
    deliveries: &[SteeringDeliveryRecord],
) -> Result<HashSet<MessageId>> {
    let allowed = deliveries
        .iter()
        .map(|delivery| delivery.message.id)
        .collect::<HashSet<_>>();

    for delivery in deliveries {
        if output
            .new_messages
            .iter()
            .chain(output.history_replacement.iter().flatten())
            .any(|message| message.id == delivery.message.id)
        {
            continue;
        }
        let target = delivery.before_message_id.ok_or_else(|| {
            anyhow!(
                "steering message {} was delivered without a terminal model response",
                delivery.message.id
            )
        })?;
        if let Some(index) = output
            .new_messages
            .iter()
            .position(|message| message.id == target)
        {
            output.new_messages.insert(index, delivery.message.clone());
            continue;
        }
        if let Some(replacement) = output.history_replacement.as_mut()
            && let Some(index) = replacement.iter().position(|message| message.id == target)
        {
            replacement.insert(index, delivery.message.clone());
            continue;
        }
        return Err(anyhow!(
            "workflow output dropped steering response anchor {target}"
        ));
    }
    Ok(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dropped_run_guard_releases_reservation_and_queue() {
        let queue = Arc::new(SessionSteering::default());
        assert!(matches!(
            queue.reserve("initial".to_owned()).await.expect("reserve"),
            UserMessageReservation::Start(_)
        ));
        assert!(matches!(
            queue.reserve("queued".to_owned()).await.expect("queue"),
            UserMessageReservation::Queued(_)
        ));

        let guard = queue.run_guard();
        drop(guard);

        assert!(queue.queued_messages().await.is_empty());
        assert!(matches!(
            queue
                .reserve("after abort".to_owned())
                .await
                .expect("reserve after abort"),
            UserMessageReservation::Start(_)
        ));
    }

    #[tokio::test]
    async fn queue_enforces_message_and_byte_limits() {
        let queue = SessionSteering::default();
        assert!(matches!(
            queue.reserve("initial".to_owned()).await.expect("reserve"),
            UserMessageReservation::Start(_)
        ));
        for index in 0..MAX_QUEUED_MESSAGES {
            assert!(matches!(
                queue
                    .reserve(format!("queued {index}"))
                    .await
                    .expect("queue within count limit"),
                UserMessageReservation::Queued(_)
            ));
        }
        let error = queue
            .reserve("one too many".to_owned())
            .await
            .expect_err("queue count must be bounded");
        assert!(error.to_string().contains("queue is full"));

        queue.abort().await;
        assert!(matches!(
            queue.reserve("initial".to_owned()).await.expect("reserve"),
            UserMessageReservation::Start(_)
        ));
        let max_message = "x".repeat(MAX_MESSAGE_BYTES);
        for _ in 0..2 {
            assert!(matches!(
                queue
                    .reserve(max_message.clone())
                    .await
                    .expect("queue within byte limit"),
                UserMessageReservation::Queued(_)
            ));
        }
        let error = queue
            .reserve("x".to_owned())
            .await
            .expect_err("queue bytes must be bounded");
        assert!(error.to_string().contains("byte budget exceeded"));
        let error = validate_message(&"x".repeat(MAX_MESSAGE_BYTES + 1))
            .expect_err("single message must be bounded");
        assert!(error.to_string().contains("byte limit"));
    }

    #[test]
    fn output_weave_preserves_queue_order_before_response() {
        let response = CanonicalMessage::text(MessageRole::Assistant, "done");
        let first = CanonicalMessage::text(MessageRole::User, "first");
        let second = CanonicalMessage::text(MessageRole::User, "second");
        let deliveries = vec![
            SteeringDeliveryRecord {
                message: first.clone(),
                before_message_id: Some(response.id),
                awaiting_anchor: false,
            },
            SteeringDeliveryRecord {
                message: second.clone(),
                before_message_id: Some(response.id),
                awaiting_anchor: false,
            },
        ];
        let mut output = WorkflowOutput::new(
            crate::domain::AgentOutput::text("done"),
            vec![response.clone()],
        );

        let allowed = weave_deliveries_into_output(&mut output, &deliveries).expect("weave");

        assert_eq!(
            output.new_messages,
            vec![first.clone(), second.clone(), response]
        );
        assert_eq!(allowed, HashSet::from([first.id, second.id]));
    }
}
