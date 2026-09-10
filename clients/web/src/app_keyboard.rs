use leptos::{html, prelude::*};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{KeyboardEvent, window};

use crate::actions::cancel_active_run;
use crate::types::TransportStatus;

#[allow(clippy::too_many_arguments)]
pub(crate) fn install_global_keydown(
    composer_ref: NodeRef<html::Textarea>,
    resize: crate::app_resize::AppResizeState,
    active_session_dir: ReadSignal<Option<String>>,
    transcript_generation: ReadSignal<u64>,
    active_run_id: ReadSignal<Option<String>>,
    next_request_id: ReadSignal<u64>,
    set_next_request_id: WriteSignal<u64>,
    set_messages: crate::transcript::TranscriptWriter,
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
            } else if ev.key() == "Escape" {
                // Сначала закрывается открытое меню композера; отмена хода —
                // только когда закрывать нечего, иначе Escape по меню
                // неожиданно стопит агента.
                if crate::app::menus::dismiss_top_menu() {
                    ev.prevent_default();
                    return;
                }
                if resize.info_open.get()
                    && window()
                        .and_then(|window| window.inner_width().ok())
                        .and_then(|width| width.as_f64())
                        .is_some_and(|width| width <= 900.0)
                {
                    ev.prevent_default();
                    resize.toggle_info_panel();
                    return;
                }
                if active_run_id.get().is_some() {
                    ev.prevent_default();
                    cancel_active_run(
                        active_session_dir,
                        transcript_generation,
                        active_run_id,
                        next_request_id,
                        set_next_request_id,
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        set_transport_status,
                    );
                }
            }
        }));
    if let Some(window) = window() {
        let _ = window
            .add_event_listener_with_callback("keydown", global_keydown.as_ref().unchecked_ref());
    }
    let listener = StoredValue::new_local(global_keydown);
    on_cleanup(move || {
        listener.with_value(|listener| {
            if let Some(window) = window() {
                let _ = window.remove_event_listener_with_callback(
                    "keydown",
                    listener.as_ref().unchecked_ref(),
                );
            }
        })
    });
}
