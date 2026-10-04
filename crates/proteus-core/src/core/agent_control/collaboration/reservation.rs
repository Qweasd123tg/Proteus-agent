use super::{SessionId, control::CollaborationControl};

pub(super) struct LaunchReservation {
    control: CollaborationControl,
    session: SessionId,
    path: String,
    generation: Option<u64>,
    transferred: bool,
}

impl LaunchReservation {
    pub(super) fn new(
        control: CollaborationControl,
        session: SessionId,
        path: String,
        generation: Option<u64>,
    ) -> Self {
        Self {
            control,
            session,
            path,
            generation,
            transferred: false,
        }
    }
    pub(super) fn transfer(&mut self) {
        self.transferred = true;
    }
}

impl Drop for LaunchReservation {
    fn drop(&mut self) {
        if self.transferred {
            return;
        }
        if let Some(generation) = self.generation {
            self.control
                .abort_followup(self.session, &self.path, generation);
        } else {
            self.control.release_reservation(self.session, &self.path);
        }
    }
}
