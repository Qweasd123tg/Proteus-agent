use leptos::{html, prelude::*};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{KeyboardEvent, window};

use crate::actions::cancel_active_turn;
use crate::types::{Message, TransportStatus};

#[allow(clippy::too_many_arguments)]
pub(crate) fn install_global_keydown(
    composer_ref: NodeRef<html::Textarea>,
    active_turn_id: ReadSignal<Option<String>>,
    next_request_id: ReadSignal<u64>,
    set_next_request_id: WriteSignal<u64>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    let global_keydown =
        Closure::<dyn FnMut(KeyboardEvent)>::wrap(Box::new(move |ev: KeyboardEvent| {
            if ev.ctrl_key() && ev.key().eq_ignore_ascii_case("l") {
                ev.prevent_default();
                if let Some(textarea) = composer_ref.get() {
                    let _ = textarea.focus();
                }
            } else if ev.key() == "Escape" && active_turn_id.get().is_some() {
                ev.prevent_default();
                cancel_active_turn(
                    active_turn_id,
                    next_request_id,
                    set_next_request_id,
                    set_is_sending,
                    set_active_turn_id,
                    set_messages,
                    next_message_id,
                    set_next_message_id,
                    set_transport_status,
                );
            }
        }));
    if let Some(window) = window() {
        let _ = window
            .add_event_listener_with_callback("keydown", global_keydown.as_ref().unchecked_ref());
    }
    global_keydown.forget();
}
