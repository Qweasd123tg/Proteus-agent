//! Последнее состояние очереди публикуется под её mutation lock. Watch
//! сохраняет его даже без читателей и не зависит от доставки runtime events.

use tokio::sync::watch;

use super::{SessionSteering, SteeringQueueState};
use crate::domain::MessageId;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct QueuedMessagesSnapshot {
    pub(crate) revision: u64,
    pub(crate) messages: Vec<(MessageId, String)>,
}

impl SessionSteering {
    pub(crate) fn subscribe_queue(&self) -> watch::Receiver<QueuedMessagesSnapshot> {
        self.snapshots.subscribe()
    }

    pub(super) fn publish_queue_snapshot(&self, state: &SteeringQueueState) {
        let messages = state
            .queued
            .iter()
            .map(|queued| (queued.message.id, queued.text.clone()))
            .collect::<Vec<_>>();
        self.snapshots.send_if_modified(|snapshot| {
            if snapshot.messages == messages {
                return false;
            }
            snapshot.revision = snapshot.revision.checked_add(1).expect("queue revision");
            snapshot.messages = messages;
            true
        });
    }
}
