//! Transport-neutral pending projection. State, revision and publication share
//! one lock; readers never assemble a snapshot from independently sampled maps.

use std::sync::{Arc, Mutex};

use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use super::{AppPendingRequests, AppQueuedUserMessage, AppServerEvent};
use crate::{
    core::{BroadcastEventSink, QueuedMessagesSnapshot},
    domain::{EventEnvelope, SessionId},
};

#[derive(Clone)]
pub(super) struct AppEventPublisher(Arc<Inner>);

struct Inner {
    events: broadcast::Sender<AppServerEvent>,
    pending: Mutex<Pending>,
    updates: watch::Sender<AppPendingRequests>,
    queue: Option<watch::Receiver<QueuedMessagesSnapshot>>,
}

struct Pending {
    snapshot: AppPendingRequests,
    queue_revision: u64,
}

/// Both transports receive an initial snapshot and subsequent complete
/// revisions through this subscription. Slow subscribers coalesce pending
/// updates while ordinary runtime events retain their bounded broadcast ring.
pub(crate) struct AppSubscription {
    events: broadcast::Receiver<AppServerEvent>,
    pending: watch::Receiver<AppPendingRequests>,
    initial: bool,
    pending_open: bool,
}

impl AppSubscription {
    pub(crate) async fn recv(&mut self) -> Result<AppServerEvent, broadcast::error::RecvError> {
        if self.initial {
            self.initial = false;
            return Ok(self.snapshot_event());
        }
        loop {
            tokio::select! {
                biased;
                changed = self.pending.changed(), if self.pending_open => {
                    if changed.is_ok() {
                        return Ok(self.snapshot_event());
                    }
                    self.pending_open = false;
                }
                event = self.events.recv() => return event,
            }
        }
    }

    fn snapshot_event(&mut self) -> AppServerEvent {
        AppServerEvent::PendingRequestsUpdated {
            snapshot: Box::new(self.pending.borrow_and_update().clone()),
        }
    }
}

impl AppEventPublisher {
    pub(super) fn new(
        capacity: usize,
        session_id: SessionId,
        queue: Option<watch::Receiver<QueuedMessagesSnapshot>>,
    ) -> Self {
        let snapshot = AppPendingRequests::new(session_id, Uuid::new_v4().to_string());
        let publisher = Self(Arc::new(Inner {
            events: broadcast::channel(capacity).0,
            pending: Mutex::new(Pending {
                snapshot: snapshot.clone(),
                queue_revision: 0,
            }),
            updates: watch::channel(snapshot).0,
            queue: queue.clone(),
        }));
        if let Some(mut queue) = queue {
            let events = publisher.clone();
            tokio::spawn(async move {
                while queue.changed().await.is_ok() {
                    // Read the latest value under the projection lock, never
                    // replay a possibly superseded notification payload.
                    events.pending_snapshot();
                }
            });
        }
        publisher
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<AppServerEvent> {
        self.0.events.subscribe()
    }

    pub(super) fn subscribe_with_pending(&self) -> AppSubscription {
        // Subscribe first, then sample: events <= initial seq may still be
        // observed, but no update after that baseline can be missed.
        let events = self.subscribe();
        AppSubscription {
            events,
            pending: self.subscribe_pending(),
            initial: true,
            pending_open: true,
        }
    }

    pub(super) fn subscribe_pending(&self) -> watch::Receiver<AppPendingRequests> {
        self.pending_snapshot();
        self.0.updates.subscribe()
    }

    pub(super) fn pending_snapshot(&self) -> AppPendingRequests {
        let mut pending = self.0.pending.lock().expect("pending projection lock");
        if self.sync_queue(&mut pending) {
            self.publish_pending(&mut pending);
        }
        pending.snapshot.clone()
    }

    pub(super) fn send(
        &self,
        event: AppServerEvent,
    ) -> Result<usize, broadcast::error::SendError<AppServerEvent>> {
        let mut pending = self.0.pending.lock().expect("pending projection lock");
        let queue_changed = self.sync_queue(&mut pending);
        let changed = match &event {
            AppServerEvent::ApprovalRequested { request } => {
                let items = &mut pending.snapshot.approvals;
                items.retain(|item| item.approval_id != request.approval_id);
                items.push((**request).clone());
                items.sort_by(|a, b| a.seq.cmp(&b.seq).then(a.approval_id.cmp(&b.approval_id)));
                true
            }
            AppServerEvent::ApprovalResolved { approval_id, .. } => {
                let items = &mut pending.snapshot.approvals;
                let before = items.len();
                items.retain(|item| &item.approval_id != approval_id);
                items.len() != before
            }
            AppServerEvent::UserInputRequested { request } => {
                let items = &mut pending.snapshot.user_inputs;
                items.retain(|item| item.request_id != request.request_id);
                items.push((**request).clone());
                items.sort_by(|a, b| a.seq.cmp(&b.seq).then(a.request_id.cmp(&b.request_id)));
                true
            }
            AppServerEvent::UserInputResolved { request_id } => {
                let items = &mut pending.snapshot.user_inputs;
                let before = items.len();
                items.retain(|item| &item.request_id != request_id);
                items.len() != before
            }
            _ => false,
        };
        let result = self.0.events.send(event);
        if changed || queue_changed {
            self.publish_pending(&mut pending);
        }
        result
    }

    fn sync_queue(&self, pending: &mut Pending) -> bool {
        let Some(queue) = &self.0.queue else {
            return false;
        };
        let queue = queue.borrow();
        if queue.revision == pending.queue_revision {
            return false;
        }
        pending.queue_revision = queue.revision;
        pending.snapshot.queued_user_messages = queue
            .messages
            .iter()
            .map(|(id, text)| AppQueuedUserMessage::new(*id, text.clone()))
            .collect();
        true
    }

    fn publish_pending(&self, pending: &mut Pending) {
        pending.snapshot.seq = pending
            .snapshot
            .seq
            .checked_add(1)
            .expect("pending sequence");
        // Complete snapshots need only one retained value, regardless of
        // subscriber speed. Intermediate revisions can safely be coalesced.
        self.0.updates.send_replace(pending.snapshot.clone());
    }
}

#[cfg(test)]
pub(super) fn test_event_channel(
    capacity: usize,
) -> (AppEventPublisher, broadcast::Receiver<AppServerEvent>) {
    let events = AppEventPublisher::new(capacity, crate::domain::new_session_id(), None);
    let receiver = events.subscribe();
    (events, receiver)
}

pub(super) fn spawn_runtime_event_forwarder(
    core_broadcast: Arc<BroadcastEventSink>,
    events: AppEventPublisher,
    turn_progress: Arc<tokio::sync::Mutex<super::TurnProgress>>,
) {
    let rx = core_broadcast.subscribe();
    spawn_runtime_event_forwarder_with_receiver(rx, events, turn_progress);
}

/// Отделено от `spawn_runtime_event_forwarder`, чтобы lag-путь можно было
/// детерминированно тестировать: receiver подписывается до переполнения.
pub(super) fn spawn_runtime_event_forwarder_with_receiver(
    mut rx: broadcast::Receiver<EventEnvelope>,
    events: AppEventPublisher,
    turn_progress: Arc<tokio::sync::Mutex<super::TurnProgress>>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    turn_progress.lock().await.apply(&envelope);
                    let _ = events.send(AppServerEvent::Runtime {
                        envelope: Box::new(envelope),
                    });
                }
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    // Часть runtime-событий потеряна (переполнение ring):
                    // среди них могли быть ToolFinished/TurnFinished. Клиент
                    // обязан пересинхронизироваться, а не жить со «вечно
                    // бегущими» карточками — Error здесь не подходит, он
                    // означает «ход упал».
                    let _ = events.send(AppServerEvent::EventStreamLagged { count });
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
