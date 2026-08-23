use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use proteus_process_host::ReceiveLimits;
use serde_json::Value;
use tokio::sync::mpsc;

pub type InvocationNotificationReceiver = mpsc::Receiver<InvocationNotification>;

pub struct InvocationNotification {
    pub method: String,
    pub params: Value,
    _permit: NotificationPermit,
}

impl fmt::Debug for InvocationNotification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InvocationNotification")
            .field("method", &self.method)
            .field("params", &self.params)
            .finish()
    }
}

impl PartialEq for InvocationNotification {
    fn eq(&self, other: &Self) -> bool {
        self.method == other.method && self.params == other.params
    }
}

#[derive(Debug, Default)]
struct BudgetState {
    frames: usize,
    bytes: usize,
}

#[derive(Debug)]
struct NotificationBudget {
    limits: ReceiveLimits,
    state: Mutex<BudgetState>,
}

impl NotificationBudget {
    fn reserve(self: &Arc<Self>, bytes: usize) -> Option<NotificationPermit> {
        let mut state = self
            .state
            .lock()
            .expect("notification budget mutex poisoned");
        let frames = state.frames.saturating_add(1);
        let total_bytes = state.bytes.saturating_add(bytes);
        if frames > self.limits.max_buffered_frames()
            || total_bytes > self.limits.max_buffered_bytes()
        {
            return None;
        }
        state.frames = frames;
        state.bytes = total_bytes;
        Some(NotificationPermit {
            bytes,
            budget: Arc::clone(self),
        })
    }

    fn release(&self, bytes: usize) {
        let mut state = self
            .state
            .lock()
            .expect("notification budget mutex poisoned");
        state.frames = state.frames.saturating_sub(1);
        state.bytes = state.bytes.saturating_sub(bytes);
    }
}

struct NotificationPermit {
    bytes: usize,
    budget: Arc<NotificationBudget>,
}

impl Drop for NotificationPermit {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

pub(crate) enum NotificationDelivery {
    Delivered,
    Dropped,
}

#[derive(Clone, Debug)]
pub(crate) struct NotificationSink {
    sender: mpsc::Sender<InvocationNotification>,
    budget: Arc<NotificationBudget>,
    dropped: Arc<AtomicU64>,
}

impl NotificationSink {
    pub(crate) fn channel(limits: ReceiveLimits) -> (Self, InvocationNotificationReceiver) {
        let (sender, receiver) = mpsc::channel(limits.max_buffered_frames());
        (
            Self {
                sender,
                budget: Arc::new(NotificationBudget {
                    limits,
                    state: Mutex::new(BudgetState::default()),
                }),
                dropped: Arc::new(AtomicU64::new(0)),
            },
            receiver,
        )
    }

    pub(crate) fn try_send(
        &self,
        method: String,
        params: Value,
        bytes: usize,
    ) -> NotificationDelivery {
        let Some(permit) = self.budget.reserve(bytes) else {
            self.dropped.fetch_add(1, Ordering::AcqRel);
            return NotificationDelivery::Dropped;
        };
        let notification = InvocationNotification {
            method,
            params,
            _permit: permit,
        };
        if self.sender.try_send(notification).is_err() {
            self.dropped.fetch_add(1, Ordering::AcqRel);
            return NotificationDelivery::Dropped;
        }
        NotificationDelivery::Delivered
    }

    pub(crate) fn dropped_counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.dropped)
    }
}
