//! Runtime-owned live-history notification, independent of any tool facade.
//!
//! A backend expires the shared retention token when its resume binding is
//! removed. Observers keep that token, not a completion-time boolean, so a
//! late completion cannot revive an expired history.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::{contracts::AgentAddress, domain::SessionId};

#[derive(Clone, Debug)]
pub(super) struct HistoryRetention(Arc<AtomicBool>);

impl HistoryRetention {
    pub(super) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    pub(super) fn is_retained(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(super) fn expire(&self) {
        self.0.store(false, Ordering::Release);
    }

    pub(super) fn same_history(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

type HistoryObserver = dyn Fn(SessionId, &AgentAddress, HistoryRetention) + Send + Sync;

pub(super) struct HistoryNotifier(Arc<HistoryObserver>);

impl HistoryNotifier {
    pub(super) fn new(
        observer: impl Fn(SessionId, &AgentAddress, HistoryRetention) + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(observer))
    }

    pub(super) fn bind(
        &self,
        session_id: SessionId,
        target: &AgentAddress,
        history: HistoryRetention,
    ) {
        (self.0)(session_id, target, history);
    }
}
