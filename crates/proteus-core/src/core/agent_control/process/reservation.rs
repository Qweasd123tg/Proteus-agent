use super::*;

/// Owns both reservations until a detached execution and its monitor are attached.
pub(super) struct SpawnReservation {
    owner: Arc<RunnerInner>,
    pending: Option<String>,
    resume: Option<ResumeReservation>,
}

impl SpawnReservation {
    pub(super) fn new(owner: Arc<RunnerInner>, resume: Option<ResumeReservation>) -> Self {
        Self {
            owner,
            pending: None,
            resume,
        }
    }
    pub(super) fn reserve_pending(&mut self, id: String) {
        self.pending = Some(id);
    }
    pub(super) fn transfer(&mut self) {
        self.pending = None;
        self.resume = None;
    }
}

impl Drop for SpawnReservation {
    fn drop(&mut self) {
        if let Some(id) = self.pending.take() {
            if let Ok(mut pending) = self.owner.lock_pending() {
                pending.release(&id);
            }
        }
        if let Some(resume) = self.resume.take() {
            if let Ok(mut pool) = self.owner.lock_pool() {
                if let Ok(outcome) = pool.cancel_reservation(&resume) {
                    // Dropping each evicted ChildProcess kills its child. Retained
                    // reservations are back in the idle pool before Drop returns.
                    drop(outcome.evicted);
                }
            }
        }
    }
}
