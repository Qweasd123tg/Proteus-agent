mod runtime;
mod stream;

use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Event, EventSource, MessageEvent};

use crate::actions::handle_command_response;
use crate::api::{event_stream_url, get_json, js_error};
use crate::app_helpers::{
    apply_active_session_activity, load_sidebar_sessions, replace_transcript,
};
use self::runtime::{
    event_updates_visible_count, update_runtime_status_and_tools, update_session_labels,
};
pub(crate) use self::stream::BufferedStreamDeltas;
use self::stream::{StreamFlushBindings, flush_stream_delta_buffer};
use crate::messages::{
    finish_all_streaming_assistant_messages, finish_streaming_assistant_message,
    push_assistant_message_if_missing, push_message, push_user_message_once,
};
use crate::types::*;
use crate::ui_utils::output_text;

#[derive(Clone, Copy)]
pub(crate) struct EventStreamBindings {
    pub(crate) set_messages: WriteSignal<Vec<Message>>,
    pub(crate) next_message_id: ReadSignal<u64>,
    pub(crate) set_next_message_id: WriteSignal<u64>,
    pub(crate) transport_status: ReadSignal<TransportStatus>,
    pub(crate) set_transport_status: WriteSignal<TransportStatus>,
    pub(crate) set_event_count: WriteSignal<u64>,
    pub(crate) set_workspace_label: WriteSignal<String>,
    pub(crate) set_session_label: WriteSignal<String>,
    pub(crate) active_session_dir: ReadSignal<Option<String>>,
    pub(crate) set_active_session_dir: WriteSignal<Option<String>>,
    pub(crate) set_is_sending: WriteSignal<bool>,
    pub(crate) set_active_turn_id: WriteSignal<Option<String>>,
    pub(crate) active_stream_message_id: ReadSignal<Option<u64>>,
    pub(crate) set_active_stream_message_id: WriteSignal<Option<u64>>,
    pub(crate) streamed_this_turn: ReadSignal<bool>,
    pub(crate) set_streamed_this_turn: WriteSignal<bool>,
    pub(crate) stream_delta_buffer: StoredValue<BufferedStreamDeltas, LocalStorage>,
    pub(crate) set_agent_status: WriteSignal<String>,
    pub(crate) set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    pub(crate) set_context_usage: WriteSignal<Option<ContextUsage>>,
    pub(crate) transcript_generation: ReadSignal<u64>,
    pub(crate) set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    pub(crate) set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
    pub(crate) set_sidebar_sessions: WriteSignal<Vec<SessionSummary>>,
    pub(crate) set_sidebar_sessions_status: WriteSignal<String>,
}

pub(crate) fn reconnect_event_stream(
    event_source: StoredValue<Option<EventSource>, LocalStorage>,
    bindings: EventStreamBindings,
) {
    event_source.update_value(|slot| {
        bindings
            .set_transport_status
            .set(TransportStatus::Connecting);
        if let Some(source) = slot.take() {
            source.close();
        }
        *slot = connect_event_stream(bindings);
    });
}

pub(crate) fn close_event_stream(event_source: StoredValue<Option<EventSource>, LocalStorage>) {
    event_source.update_value(|slot| {
        if let Some(source) = slot.take() {
            source.close();
        }
    });
}

fn connect_event_stream(bindings: EventStreamBindings) -> Option<EventSource> {
    let url = event_stream_url();
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

    let on_open = Closure::<dyn FnMut(Event)>::wrap(Box::new(move |_| {
        if bindings.transcript_generation.get_untracked() != stream_generation {
            return;
        }
        let was_disconnected = matches!(
            bindings.transport_status.get_untracked(),
            TransportStatus::Error(_)
        );
        bindings
            .set_transport_status
            .set(TransportStatus::Connected);
        refresh_pending_control_plane(
            bindings.set_pending_approvals,
            bindings.set_pending_user_inputs,
        );
        if was_disconnected {
            // События за время обрыва потеряны: стрим-состояние невалидно,
            // транскрипт перечитывается с сервера целиком.
            bindings
                .stream_delta_buffer
                .set_value(BufferedStreamDeltas::default());
            bindings.set_active_stream_message_id.set(None);
            bindings.set_streamed_this_turn.set(false);
            let expected_generation = bindings.transcript_generation.get_untracked();
            replace_transcript(
                bindings.set_messages,
                bindings.transcript_generation,
                expected_generation,
                bindings.next_message_id,
                bindings.set_next_message_id,
                bindings.set_transport_status,
            );
        }
    }));
    source.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let output_messages = bindings.set_messages;
    let output_next_message_id = bindings.next_message_id;
    let output_set_next_message_id = bindings.set_next_message_id;
    let output_transport_status = bindings.set_transport_status;
    let output_event_count = bindings.set_event_count;
    let on_output =
        Closure::<dyn FnMut(MessageEvent)>::wrap(Box::new(move |event: MessageEvent| {
            if bindings.transcript_generation.get_untracked() != stream_generation {
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
                    bindings.set_active_session_dir,
                    bindings.set_is_sending,
                    bindings.set_active_turn_id,
                    bindings.active_stream_message_id,
                    bindings.set_active_stream_message_id,
                    bindings.streamed_this_turn,
                    bindings.set_streamed_this_turn,
                    bindings.stream_delta_buffer,
                    bindings.set_agent_status,
                    bindings.set_tool_activities,
                    bindings.set_context_usage,
                    bindings.set_pending_approvals,
                    bindings.set_pending_user_inputs,
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
    on_output.forget();

    let set_transport_status = bindings.set_transport_status;
    let transcript_generation = bindings.transcript_generation;
    let on_error = Closure::<dyn FnMut(Event)>::wrap(Box::new(move |_| {
        if transcript_generation.get_untracked() != stream_generation {
            return;
        }
        set_transport_status.set(TransportStatus::Error(
            "event stream disconnected".to_owned(),
        ));
    }));
    source.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();

    Some(source)
}

fn refresh_pending_control_plane(
    set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
) {
    spawn_local(async move {
        match get_json::<PendingControlPlaneInfo>("/pending").await {
            Ok(pending) => {
                set_pending_approvals.set(pending.approvals);
                set_pending_user_inputs.set(pending.user_inputs);
            }
            Err(error) => {
                web_sys::console::warn_1(&JsValue::from_str(&format!(
                    "Pending control-plane refresh failed: {error}"
                )));
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn handle_app_output(
    output: StdioOutput,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
    set_event_count: WriteSignal<u64>,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
    active_session_dir: ReadSignal<Option<String>>,
    set_active_session_dir: WriteSignal<Option<String>>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    streamed_this_turn: ReadSignal<bool>,
    set_streamed_this_turn: WriteSignal<bool>,
    stream_delta_buffer: StoredValue<BufferedStreamDeltas, LocalStorage>,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_context_usage: WriteSignal<Option<ContextUsage>>,
    set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
    set_sidebar_sessions: WriteSignal<Vec<SessionSummary>>,
    set_sidebar_sessions_status: WriteSignal<String>,
) {
    match output {
        StdioOutput::Event { event } => {
            if event_updates_visible_count(&event) {
                set_event_count.update(|count| *count += 1);
            }
            handle_app_event(
                *event,
                set_messages,
                next_message_id,
                set_next_message_id,
                set_transport_status,
                set_workspace_label,
                set_session_label,
                active_session_dir,
                set_active_session_dir,
                set_is_sending,
                set_active_turn_id,
                active_stream_message_id,
                set_active_stream_message_id,
                streamed_this_turn,
                set_streamed_this_turn,
                stream_delta_buffer,
                set_agent_status,
                set_tool_activities,
                set_context_usage,
                set_pending_approvals,
                set_pending_user_inputs,
                set_sidebar_sessions,
                set_sidebar_sessions_status,
            );
        }
        StdioOutput::Response { .. } => handle_command_response(
            output,
            set_messages,
            next_message_id,
            set_next_message_id,
            set_transport_status,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_app_event(
    event: AppServerEvent,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
    active_session_dir: ReadSignal<Option<String>>,
    set_active_session_dir: WriteSignal<Option<String>>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    streamed_this_turn: ReadSignal<bool>,
    set_streamed_this_turn: WriteSignal<bool>,
    stream_delta_buffer: StoredValue<BufferedStreamDeltas, LocalStorage>,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_context_usage: WriteSignal<Option<ContextUsage>>,
    set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
    set_sidebar_sessions: WriteSignal<Vec<SessionSummary>>,
    set_sidebar_sessions_status: WriteSignal<String>,
) {
    let stream_bindings = StreamFlushBindings {
        set_messages,
        next_message_id,
        set_next_message_id,
        active_stream_message_id,
        set_active_stream_message_id,
        streamed_this_turn,
        set_streamed_this_turn,
        stream_delta_buffer,
    };
    match event {
        AppServerEvent::Runtime { envelope } => {
            update_runtime_status_and_tools(
                &envelope,
                set_messages,
                next_message_id,
                set_next_message_id,
                stream_bindings,
                set_agent_status,
                set_tool_activities,
                set_context_usage,
            );
            update_session_labels(
                envelope,
                set_workspace_label,
                set_session_label,
                set_active_session_dir,
            );
        }
        AppServerEvent::UserMessageSubmitted { text } => {
            flush_stream_delta_buffer(stream_bindings);
            set_streamed_this_turn.set(false);
            set_active_stream_message_id.set(None);
            push_user_message_once(set_messages, next_message_id, set_next_message_id, text);
        }
        AppServerEvent::TurnOutput { output } => {
            flush_stream_delta_buffer(stream_bindings);
            set_is_sending.set(false);
            set_active_turn_id.set(None);
            set_agent_status.set("ожидает".to_owned());
            let final_text = non_empty_output_text(&output);
            if streamed_this_turn.get() {
                if let Some(message_id) = active_stream_message_id.get() {
                    set_messages.update(|items| {
                        if let Some(message) =
                            items.iter_mut().find(|message| message.id == message_id)
                        {
                            message.streaming = false;
                            message.version += 1;
                        }
                    });
                } else {
                    finish_all_streaming_assistant_messages(set_messages);
                }
                set_active_stream_message_id.set(None);
                set_streamed_this_turn.set(false);
                if let Some(final_text) = final_text {
                    push_assistant_message_if_missing(
                        set_messages,
                        next_message_id,
                        set_next_message_id,
                        final_text,
                    );
                }
            } else {
                finish_streaming_assistant_message(
                    set_messages,
                    next_message_id,
                    set_next_message_id,
                    active_stream_message_id,
                    set_active_stream_message_id,
                    output_text(&output),
                );
            }
            load_sidebar_sessions(set_sidebar_sessions, set_sidebar_sessions_status);
        }
        AppServerEvent::ApprovalRequested { request } => {
            let request = *request;
            set_agent_status.set("ждёт доступ".to_owned());
            set_pending_approvals.update(|items| {
                if let Some(item) = items
                    .iter_mut()
                    .find(|item| item.approval_id == request.approval_id)
                {
                    *item = request;
                } else {
                    items.push(request);
                }
            });
        }
        AppServerEvent::ApprovalResolved {
            approval_id,
            approved,
        } => {
            set_agent_status.set(if approved {
                "доступ разрешён".to_owned()
            } else {
                "доступ отклонён".to_owned()
            });
            set_pending_approvals
                .update(|items| items.retain(|item| item.approval_id != approval_id));
        }
        AppServerEvent::UserInputRequested { request } => {
            let request = *request;
            set_agent_status.set("ждёт ответ".to_owned());
            set_pending_user_inputs.update(|items| {
                if let Some(item) = items
                    .iter_mut()
                    .find(|item| item.request_id == request.request_id)
                {
                    *item = request;
                } else {
                    items.push(request);
                }
            });
        }
        AppServerEvent::UserInputResolved { request_id } => {
            set_agent_status.set("продолжает".to_owned());
            set_pending_user_inputs.update(|items| {
                items.retain(|item| item.request_id != request_id);
            });
        }
        AppServerEvent::SessionActivityUpdated {
            session_dir,
            activity,
        } => {
            if active_session_dir.get_untracked().as_deref() == Some(session_dir.as_str()) {
                apply_active_session_activity(
                    Some(&activity),
                    set_is_sending,
                    set_active_turn_id,
                    set_agent_status,
                );
            }
            let mut found = false;
            set_sidebar_sessions.update(|items| {
                if let Some(session) = items
                    .iter_mut()
                    .find(|session| session.session_dir == session_dir)
                {
                    session.activity = Some(activity.clone());
                    found = true;
                }
            });
            if !found {
                load_sidebar_sessions(set_sidebar_sessions, set_sidebar_sessions_status);
            }
        }
        AppServerEvent::Error { message } => {
            flush_stream_delta_buffer(stream_bindings);
            set_is_sending.set(false);
            set_active_turn_id.set(None);
            set_agent_status.set("ошибка".to_owned());
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                format!("AppServer error: {message}"),
            );
        }
        AppServerEvent::Shutdown => {
            flush_stream_delta_buffer(stream_bindings);
            set_is_sending.set(false);
            set_active_turn_id.set(None);
            set_agent_status.set("остановлено".to_owned());
            set_transport_status.set(TransportStatus::Shutdown);
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                "AppServer shutdown".to_owned(),
            );
        }
        AppServerEvent::Unknown => {}
    }
}

fn non_empty_output_text(output: &Value) -> Option<String> {
    output
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(ToOwned::to_owned)
}
