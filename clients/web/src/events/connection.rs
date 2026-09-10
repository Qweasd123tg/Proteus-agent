use super::{EventStreamBindings, handle_app_output};
use crate::{
    api::{event_stream_url, js_error},
    messages::push_message,
    types::*,
    ui_utils::set_timeout,
};
use leptos::prelude::*;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{Event, EventSource, MessageEvent};

/// Own the browser connection and all its callbacks as one disposable resource.
pub(crate) struct EventConnection {
    source: EventSource,
    _on_open: Closure<dyn FnMut(Event)>,
    on_output: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(Event)>,
    alive: std::rc::Rc<std::cell::Cell<bool>>,
}
impl Drop for EventConnection {
    fn drop(&mut self) {
        self.alive.set(false);
        self.source.set_onopen(None);
        self.source.set_onerror(None);
        let _ = self
            .source
            .remove_event_listener_with_callback("output", self.on_output.as_ref().unchecked_ref());
        self.source.close();
    }
}
/// Сколько ждём авто-реконнект EventSource, прежде чем показать ошибку.
const RECONNECT_GRACE_MS: i32 = 5000;

pub(crate) fn reconnect_event_stream(
    event_source: StoredValue<Option<EventConnection>, LocalStorage>,
    bindings: EventStreamBindings,
) {
    event_source.update_value(|slot| {
        bindings
            .set_transport_status
            .set(TransportStatus::Connecting);
        drop(slot.take());
        *slot = connect_event_stream(bindings);
    });
}

pub(crate) fn close_event_stream(event_source: StoredValue<Option<EventConnection>, LocalStorage>) {
    event_source.update_value(|slot| {
        drop(slot.take());
    });
}

fn connect_event_stream(bindings: EventStreamBindings) -> Option<EventConnection> {
    let session_dir = bindings.active_session_dir.get_untracked()?;
    let url = event_stream_url(&session_dir);
    let stream_generation = bindings.transcript_generation.get_untracked();
    let source = match EventSource::new(&url) {
        Ok(source) => source,
        Err(error) => {
            let message = js_error(error);
            bindings
                .set_transport_status
                .set(TransportStatus::Error(message.clone()));
            push_message(
                bindings.set_messages,
                bindings.next_message_id,
                bindings.set_next_message_id,
                MessageRole::System,
                format!("Event stream failed: {message}"),
            );
            return None;
        }
    };

    let alive = std::rc::Rc::new(std::cell::Cell::new(true));
    let pending = super::control_plane::PendingControlPlane::new(
        bindings,
        alive.clone(),
        session_dir.clone(),
    );
    let open_pending = pending.clone();
    let open_alive = alive.clone();
    let on_open = Closure::<dyn FnMut(Event)>::wrap(Box::new(move |_| {
        if !open_alive.get() || bindings.transcript_generation.get_untracked() != stream_generation
        {
            return;
        }
        bindings
            .set_transport_status
            .set(TransportStatus::Connected);
        open_pending.begin_connection();
        open_pending.refresh();
    }));
    source.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    let output_messages = bindings.set_messages;
    let output_next_message_id = bindings.next_message_id;
    let output_set_next_message_id = bindings.set_next_message_id;
    let output_transport_status = bindings.set_transport_status;
    let output_event_count = bindings.set_event_count;
    let output_alive = alive.clone();
    let on_output =
        Closure::<dyn FnMut(MessageEvent)>::wrap(Box::new(move |event: MessageEvent| {
            if !output_alive.get()
                || bindings.transcript_generation.get_untracked() != stream_generation
            {
                return;
            }
            let Some(data) = event.data().as_string() else {
                return;
            };
            match serde_json::from_str::<StdioOutput>(&data) {
                Ok(output) => handle_app_output(
                    output,
                    output_messages,
                    output_next_message_id,
                    output_set_next_message_id,
                    output_transport_status,
                    output_event_count,
                    bindings.set_workspace_label,
                    bindings.set_session_label,
                    bindings.active_session_dir,
                    bindings.set_is_sending,
                    bindings.set_active_run_id,
                    bindings.active_stream_message_id,
                    bindings.set_active_stream_message_id,
                    bindings.streamed_this_turn,
                    bindings.set_streamed_this_turn,
                    bindings.stream_delta_buffer,
                    bindings.set_agent_status,
                    bindings.set_tool_activities,
                    bindings.set_context_usage,
                    &pending,
                    bindings.set_sidebar_sessions,
                    bindings.set_sidebar_sessions_status,
                ),
                Err(error) => push_message(
                    output_messages,
                    output_next_message_id,
                    output_set_next_message_id,
                    MessageRole::System,
                    format!("Invalid event payload: {error}"),
                ),
            }
        }));
    let _ = source.add_event_listener_with_callback("output", on_output.as_ref().unchecked_ref());

    let set_transport_status = bindings.set_transport_status;
    let transport_status = bindings.transport_status;
    let transcript_generation = bindings.transcript_generation;
    let error_source = source.clone();
    let error_alive = alive.clone();
    let on_error = Closure::<dyn FnMut(Event)>::wrap(Box::new(move |_| {
        if !error_alive.get() || transcript_generation.get_untracked() != stream_generation {
            return;
        }
        // Терминальный обрыв: браузер ретраить не будет (например, HTTP 4xx).
        if error_source.ready_state() == EventSource::CLOSED {
            set_transport_status.set(TransportStatus::Error(
                "event stream disconnected".to_owned(),
            ));
            return;
        }
        // EventSource сам переподключается, onerror приходит на каждый
        // ретрай. Ошибку показываем только если реконнект не удался за
        // грейс-период — иначе бейдж и тост мигают при каждом коротком
        // обрыве (переключение сессий, перезапуск runtime).
        if matches!(
            transport_status.get_untracked(),
            TransportStatus::Connected | TransportStatus::Connecting
        ) {
            set_transport_status.set(TransportStatus::Reconnecting);
            let alive = error_alive.clone();
            set_timeout(RECONNECT_GRACE_MS, move || {
                if !alive.get() {
                    return;
                }
                if transcript_generation.get_untracked() != stream_generation {
                    return;
                }
                if transport_status.get_untracked() == TransportStatus::Reconnecting {
                    set_transport_status.set(TransportStatus::Error(
                        "event stream disconnected".to_owned(),
                    ));
                }
            });
        }
    }));
    source.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    Some(EventConnection {
        source,
        _on_open: on_open,
        on_output,
        _on_error: on_error,
        alive,
    })
}
