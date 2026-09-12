use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use leptos::{prelude::*, task::spawn_local};
use proteus_client_common::pending::PendingCursor;
use wasm_bindgen::JsValue;

use super::EventStreamBindings;
use crate::{
    api::{get_json, session_path},
    types::PendingControlPlaneInfo,
};

#[derive(Clone)]
pub(super) struct PendingControlPlane {
    cursor: Rc<RefCell<PendingCursor>>,
    alive: Rc<Cell<bool>>,
    session_dir: String,
    generation: u64,
    bindings: EventStreamBindings,
}

impl PendingControlPlane {
    pub(super) fn new(
        bindings: EventStreamBindings,
        alive: Rc<Cell<bool>>,
        session_dir: String,
    ) -> Self {
        Self {
            cursor: Rc::new(RefCell::new(PendingCursor::default())),
            alive,
            session_dir,
            generation: bindings.transcript_generation.get_untracked(),
            bindings,
        }
    }

    pub(super) fn begin_connection(&self) {
        self.cursor.borrow_mut().begin_connection();
    }

    fn is_current(&self) -> bool {
        self.alive.get()
            && self.bindings.transcript_generation.try_get_untracked() == Some(self.generation)
            && self
                .bindings
                .active_session_dir
                .try_get_untracked()
                .flatten()
                .as_deref()
                == Some(self.session_dir.as_str())
    }

    pub(super) fn apply_stream(&self, snapshot: PendingControlPlaneInfo) {
        if self.is_current()
            && self.cursor.borrow_mut().accept_stream(
                &snapshot.session_id.to_string(),
                &snapshot.stream_id,
                snapshot.seq,
            )
        {
            self.apply(snapshot);
        }
    }

    pub(super) fn refresh(&self) {
        let this = self.clone();
        let ticket = self.cursor.borrow().read_ticket();
        spawn_local(async move {
            let result =
                get_json::<PendingControlPlaneInfo>(&session_path("/pending", &this.session_dir))
                    .await;
            if !this.is_current() {
                return;
            }
            match result {
                Ok(snapshot) => {
                    if this.cursor.borrow_mut().accept_read(
                        ticket,
                        &snapshot.session_id.to_string(),
                        &snapshot.stream_id,
                        snapshot.seq,
                    ) {
                        this.apply(snapshot);
                    }
                }
                Err(error) => web_sys::console::warn_1(&JsValue::from_str(&format!(
                    "Pending refresh failed: {error}"
                ))),
            }
        });
    }

    fn apply(&self, snapshot: PendingControlPlaneInfo) {
        self.bindings.set_agent_status.update(|status| {
            if !snapshot.approvals.is_empty() {
                *status = "ждёт доступ".to_owned();
            } else if !snapshot.user_inputs.is_empty() {
                *status = "ждёт ответ".to_owned();
            } else if matches!(status.as_str(), "ждёт доступ" | "ждёт ответ") {
                *status = "продолжает".to_owned();
            }
        });
        self.bindings.set_pending_approvals.set(snapshot.approvals);
        self.bindings
            .set_pending_user_inputs
            .set(snapshot.user_inputs);
        self.bindings
            .set_queued_prompts
            .set(snapshot.queued_user_messages);
    }
}
