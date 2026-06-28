use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Event, EventSource, MessageEvent};

use crate::actions::handle_command_response;
use crate::api::{event_stream_url, get_json, js_error};
use crate::app_helpers::{
    apply_active_session_activity, load_sidebar_sessions, replace_transcript, save_context_usage,
};
use crate::messages::{
    append_streaming_assistant_delta, finish_active_streaming_assistant_message,
    finish_all_streaming_assistant_messages, finish_streaming_assistant_message,
    finish_streaming_reasoning, push_assistant_message_if_missing, push_message, push_tool_message,
    push_user_message_once, update_tool_status,
};
use crate::types::*;
use crate::ui_utils::{compact_text, format_json, output_text, set_timeout, short_id, short_path};

const STREAM_DELTA_FLUSH_MS: i32 = 80;

#[derive(Default)]
pub(crate) struct BufferedStreamDeltas {
    assistant: String,
    flush_scheduled: bool,
}

#[derive(Clone, Copy)]
struct StreamFlushBindings {
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    streamed_this_turn: ReadSignal<bool>,
    set_streamed_this_turn: WriteSignal<bool>,
    stream_delta_buffer: StoredValue<BufferedStreamDeltas, LocalStorage>,
}

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

fn event_updates_visible_count(event: &AppServerEvent) -> bool {
    !matches!(
        event,
        AppServerEvent::Runtime { envelope }
            if runtime_event_is_stream_delta(envelope)
    )
}

fn runtime_event_is_stream_delta(envelope: &Value) -> bool {
    let Some(event) = envelope.get("event") else {
        return false;
    };
    event.get("AssistantTextDelta").is_some() || event.get("AssistantReasoningDelta").is_some()
}

fn update_session_labels(
    envelope: Value,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
    set_active_session_dir: WriteSignal<Option<String>>,
) {
    let Some(started) = envelope.pointer("/event/SessionStarted") else {
        return;
    };
    if let Some(cwd) = started.get("cwd").and_then(Value::as_str) {
        set_workspace_label.set(cwd.to_owned());
    }
    if let Some(session_dir) = started.get("session_dir").and_then(Value::as_str) {
        set_active_session_dir.set(Some(session_dir.to_owned()));
        set_session_label.set(short_path(session_dir));
    } else if let Some(session_id) = started.get("session_id").and_then(Value::as_str) {
        set_session_label.set(short_id(session_id).to_owned());
    }
}

fn queue_assistant_delta(bindings: StreamFlushBindings, text: &str) {
    if text.is_empty() {
        return;
    }
    if !bindings.streamed_this_turn.get_untracked() {
        bindings.set_streamed_this_turn.set(true);
    }
    let mut should_schedule = false;
    bindings.stream_delta_buffer.update_value(|buffer| {
        buffer.assistant.push_str(text);
        if !buffer.flush_scheduled {
            buffer.flush_scheduled = true;
            should_schedule = true;
        }
    });
    if should_schedule {
        schedule_stream_delta_flush(bindings);
    }
}

fn schedule_stream_delta_flush(bindings: StreamFlushBindings) {
    set_timeout(STREAM_DELTA_FLUSH_MS, move || {
        flush_stream_delta_buffer(bindings);
    });
}

fn flush_stream_delta_buffer(bindings: StreamFlushBindings) {
    let mut assistant = String::new();
    bindings.stream_delta_buffer.update_value(|buffer| {
        buffer.flush_scheduled = false;
        assistant = std::mem::take(&mut buffer.assistant);
    });

    if assistant.is_empty() {
        return;
    }

    finish_streaming_reasoning(bindings.set_messages);
    append_streaming_assistant_delta(
        bindings.set_messages,
        bindings.next_message_id,
        bindings.set_next_message_id,
        bindings.active_stream_message_id,
        bindings.set_active_stream_message_id,
        &assistant,
    );
}

#[allow(clippy::too_many_arguments)]
fn update_runtime_status_and_tools(
    envelope: &Value,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    stream_bindings: StreamFlushBindings,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_context_usage: WriteSignal<Option<ContextUsage>>,
) {
    let Some(event) = envelope.get("event") else {
        return;
    };

    if let Some(usage_event) = event.get("TokenUsageUpdated") {
        if let Some(usage) = usage_event.get("usage").and_then(parse_context_usage) {
            save_context_usage(usage);
            set_context_usage.set(Some(usage));
        }
        return;
    }

    if event.get("TurnStarted").is_some() {
        flush_stream_delta_buffer(stream_bindings);
        stream_bindings.set_streamed_this_turn.set(false);
        stream_bindings.set_active_stream_message_id.set(None);
        finish_streaming_reasoning(set_messages);
        set_agent_status.set("начинает".to_owned());
    } else if event.get("TaskReceived").is_some() {
        set_agent_status.set("готовит задачу".to_owned());
    } else if event.get("HistoryCompactionStarted").is_some() {
        set_agent_status.set("сжимает историю".to_owned());
    } else if let Some(compaction_event) = event.get("HistoryCompactionCompleted") {
        let Some(report) = compaction_event.get("report") else {
            return;
        };
        let changed = report
            .get("changed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        set_agent_status.set(if changed {
            "история сжата".to_owned()
        } else {
            "история без сжатия".to_owned()
        });
        if changed {
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                compaction_report_text(report),
            );
        }
    } else if let Some(failed_event) = event.get("HistoryCompactionFailed") {
        set_agent_status.set("сжатие не удалось".to_owned());
        let message = failed_event
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown compaction error");
        push_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            MessageRole::System,
            format!("Сжатие истории не удалось: {}", compact_text(message, 500)),
        );
    } else if event.get("ContextBuilt").is_some() {
        set_agent_status.set("собирает контекст".to_owned());
    } else if event.get("ModelRequestPrepared").is_some() {
        set_agent_status.set("думает".to_owned());
    } else if let Some(delta_event) = event.get("AssistantTextDelta") {
        if !stream_bindings.streamed_this_turn.get_untracked() {
            set_agent_status.set("пишет".to_owned());
        }
        if let Some(text) = delta_event.get("text").and_then(Value::as_str) {
            queue_assistant_delta(stream_bindings, text);
        }
    } else if event.get("AssistantReasoningDelta").is_some() {
        // Reasoning streams can be very chatty. The working indicator already
        // says "думает"; storing every chunk in the transcript makes Firefox
        // clone and re-render a growing string while the user only needs the
        // final answer.
    } else if let Some(tool_event) = event.get("ToolCallRequested") {
        flush_stream_delta_buffer(stream_bindings);
        set_agent_status.set("запускает tool".to_owned());
        finish_active_streaming_assistant_message(
            set_messages,
            stream_bindings.active_stream_message_id,
            stream_bindings.set_active_stream_message_id,
        );
        finish_streaming_reasoning(set_messages);
        if let Some(call) = tool_event.get("call") {
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_owned();
            let name = call
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_owned();
            let args = call.get("args").cloned().unwrap_or(Value::Null);
            let args_preview = format_json(&args);
            let tool = ToolActivity {
                call_id: call_id.clone(),
                name,
                args,
                args_preview,
                started_at_ms: js_sys::Date::now().max(0.0) as u64,
                status: ToolActivityStatus::Running,
                result_preview: None,
            };
            push_tool_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                tool.clone(),
            );
            set_tool_activities.update(|items| {
                if !items.iter().any(|item| item.call_id == call_id) {
                    items.push(tool);
                    trim_tool_activities(items);
                }
            });
        }
    } else if let Some(approval_event) = event.get("ApprovalRequested") {
        set_agent_status.set("ждёт доступ".to_owned());
        if let Some(call_id) = approval_event.get("call_id").and_then(Value::as_str) {
            update_tool_status(
                set_tool_activities,
                set_messages,
                call_id,
                ToolActivityStatus::WaitingApproval,
                None,
            );
        }
    } else if let Some(approval_event) = event.get("ApprovalResolved") {
        let approved = approval_event
            .get("approved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        set_agent_status.set(if approved {
            "доступ разрешён".to_owned()
        } else {
            "доступ отклонён".to_owned()
        });
        if let Some(call_id) = approval_event.get("call_id").and_then(Value::as_str) {
            update_tool_status(
                set_tool_activities,
                set_messages,
                call_id,
                if approved {
                    ToolActivityStatus::Approved
                } else {
                    ToolActivityStatus::Denied
                },
                None,
            );
        }
    } else if let Some(tool_event) = event.get("ToolFinished") {
        set_agent_status.set("tool завершён".to_owned());
        if let Some(result) = tool_event.get("result") {
            let Some(call_id) = result.get("call_id").and_then(Value::as_str) else {
                return;
            };
            let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
            let preview = tool_result_text(result);
            update_tool_status(
                set_tool_activities,
                set_messages,
                call_id,
                if ok {
                    ToolActivityStatus::Done
                } else {
                    ToolActivityStatus::Failed
                },
                Some(preview),
            );
        }
    } else if event.get("TurnFinished").is_some() {
        flush_stream_delta_buffer(stream_bindings);
        finish_streaming_reasoning(set_messages);
        set_agent_status.set("ожидает".to_owned());
    } else if event.get("Error").is_some() {
        set_agent_status.set("ошибка".to_owned());
    }
}

/// Снимок заполнения контекста: за «использовано» берём реальный замер
/// провайдера, а при его отсутствии (фаза оценки) — локальную прикидку.
/// Без известного потолка окна бублик показывать нечем — возвращаем None.
fn parse_context_usage(usage: &Value) -> Option<ContextUsage> {
    let max_tokens = usage.get("max_input_tokens").and_then(Value::as_u64)?;
    if max_tokens == 0 {
        return None;
    }
    let used_tokens = usage
        .pointer("/actual/input_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("estimated_input_tokens").and_then(Value::as_u64))
        .unwrap_or(0);
    let compaction_trigger_tokens = usage
        .get("compaction_trigger_tokens")
        .and_then(Value::as_u64)
        .map(|tokens| tokens.min(u64::from(u32::MAX)) as u32);
    Some(ContextUsage {
        used_tokens: used_tokens.min(u64::from(u32::MAX)) as u32,
        max_tokens: max_tokens.min(u64::from(u32::MAX)) as u32,
        compaction_trigger_tokens,
    })
}

fn trim_tool_activities(items: &mut Vec<ToolActivity>) {
    let overflow = items.len().saturating_sub(12);
    if overflow > 0 {
        items.drain(0..overflow);
    }
}

fn tool_result_text(result: &Value) -> String {
    if let Some(error) = result.get("error").and_then(Value::as_str)
        && !error.is_empty()
    {
        return error.to_owned();
    }

    if let Some(output) = result.get("output").and_then(Value::as_str)
        && !output.is_empty()
    {
        return output.to_owned();
    }

    if let Some(content) = result.get("content")
        && !content.as_array().is_none_or(Vec::is_empty)
    {
        return tool_content_text(content);
    }

    if result.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        "(нет вывода)".to_owned()
    } else {
        "(tool завершился без текста ошибки)".to_owned()
    }
}

fn tool_content_text(content: &Value) -> String {
    let Some(parts) = content.as_array() else {
        return format_json(content);
    };

    let rendered = parts
        .iter()
        .filter_map(tool_content_part_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        format_json(content)
    } else {
        rendered.join("\n")
    }
}

fn tool_content_part_text(part: &Value) -> Option<String> {
    match part.get("type").and_then(Value::as_str) {
        Some("text") => part
            .get("text")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        Some("json") => part.get("value").map(format_json),
        Some("image") => {
            let mime_type = part
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Some(format!("[image tool content: {mime_type}]"))
        }
        Some("binary") => {
            let mime_type = part
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Some(format!("[binary tool content: {mime_type}]"))
        }
        _ => Some(format_json(part)),
    }
}

fn non_empty_output_text(output: &Value) -> Option<String> {
    output
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn compaction_report_text(report: &Value) -> String {
    let input_messages = report
        .get("input_messages")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_messages = report
        .get("output_messages")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let original_tokens = report
        .get("original_token_estimate")
        .and_then(Value::as_u64);
    let output_tokens = report.get("output_token_estimate").and_then(Value::as_u64);
    let summary_source = report
        .get("summary_source")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    let token_text = match (original_tokens, output_tokens) {
        (Some(before), Some(after)) => format!(", ~{before} -> ~{after} tokens"),
        _ => String::new(),
    };
    format!(
        "История сжата: {input_messages} -> {output_messages} сообщений{token_text}. Summary: {summary_source}."
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tool_result_text_keeps_full_output_for_line_preview() {
        let output = "x".repeat(700);
        let result = json!({
            "call_id": "call-1",
            "ok": true,
            "output": output,
            "content": [],
            "error": null,
        });

        assert_eq!(tool_result_text(&result), "x".repeat(700));
    }

    #[test]
    fn tool_content_text_renders_all_structured_parts_without_compacting_list() {
        let content = json!([
            {"type": "text", "text": "first"},
            {"type": "json", "value": {"a": 1, "b": [2, 3]}},
            {"type": "text", "text": "third"},
            {"type": "text", "text": "fourth"},
            {"type": "text", "text": "fifth"}
        ]);

        let rendered = tool_content_text(&content);

        assert!(rendered.contains("first"));
        assert!(rendered.contains("\"b\": ["));
        assert!(rendered.contains("fifth"));
        assert!(!rendered.contains("..."));
    }
}
