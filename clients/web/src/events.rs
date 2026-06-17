use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Event, EventSource, MessageEvent};

use crate::actions::handle_command_response;
use crate::api::{event_stream_url, get_json, js_error};
use crate::app::{load_sidebar_sessions, replace_transcript};
use crate::messages::{
    append_streaming_assistant_delta, append_streaming_reasoning_delta,
    finish_active_streaming_assistant_message, finish_all_streaming_assistant_messages,
    finish_streaming_assistant_message, finish_streaming_reasoning, push_message,
    push_tool_message, push_user_message_once, update_tool_status,
};
use crate::types::*;
use crate::ui_utils::{compact_json, compact_text, output_text, short_id, short_path};

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
    pub(crate) set_active_session_dir: WriteSignal<Option<String>>,
    pub(crate) set_is_sending: WriteSignal<bool>,
    pub(crate) set_active_turn_id: WriteSignal<Option<String>>,
    pub(crate) active_stream_message_id: ReadSignal<Option<u64>>,
    pub(crate) set_active_stream_message_id: WriteSignal<Option<u64>>,
    pub(crate) streamed_this_turn: ReadSignal<bool>,
    pub(crate) set_streamed_this_turn: WriteSignal<bool>,
    pub(crate) set_agent_status: WriteSignal<String>,
    pub(crate) set_tool_activities: WriteSignal<Vec<ToolActivity>>,
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

fn connect_event_stream(bindings: EventStreamBindings) -> Option<EventSource> {
    let url = event_stream_url();
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
            bindings.set_active_stream_message_id.set(None);
            bindings.set_streamed_this_turn.set(false);
            replace_transcript(
                bindings.set_messages,
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
                    bindings.set_active_session_dir,
                    bindings.set_is_sending,
                    bindings.set_active_turn_id,
                    bindings.active_stream_message_id,
                    bindings.set_active_stream_message_id,
                    bindings.streamed_this_turn,
                    bindings.set_streamed_this_turn,
                    bindings.set_agent_status,
                    bindings.set_tool_activities,
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
    let on_error = Closure::<dyn FnMut(Event)>::wrap(Box::new(move |_| {
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

fn handle_app_output(
    output: StdioOutput,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
    set_event_count: WriteSignal<u64>,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
    set_active_session_dir: WriteSignal<Option<String>>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    streamed_this_turn: ReadSignal<bool>,
    set_streamed_this_turn: WriteSignal<bool>,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
    set_sidebar_sessions: WriteSignal<Vec<SessionSummary>>,
    set_sidebar_sessions_status: WriteSignal<String>,
) {
    match output {
        StdioOutput::Event { event } => {
            set_event_count.update(|count| *count += 1);
            handle_app_event(
                event,
                set_messages,
                next_message_id,
                set_next_message_id,
                set_transport_status,
                set_workspace_label,
                set_session_label,
                set_active_session_dir,
                set_is_sending,
                set_active_turn_id,
                active_stream_message_id,
                set_active_stream_message_id,
                streamed_this_turn,
                set_streamed_this_turn,
                set_agent_status,
                set_tool_activities,
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

fn handle_app_event(
    event: AppServerEvent,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
    set_active_session_dir: WriteSignal<Option<String>>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    streamed_this_turn: ReadSignal<bool>,
    set_streamed_this_turn: WriteSignal<bool>,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
    set_sidebar_sessions: WriteSignal<Vec<SessionSummary>>,
    set_sidebar_sessions_status: WriteSignal<String>,
) {
    match event {
        AppServerEvent::Runtime { envelope } => {
            update_runtime_status_and_tools(
                &envelope,
                set_messages,
                next_message_id,
                set_next_message_id,
                active_stream_message_id,
                set_active_stream_message_id,
                set_streamed_this_turn,
                set_agent_status,
                set_tool_activities,
            );
            update_session_labels(
                envelope,
                set_workspace_label,
                set_session_label,
                set_active_session_dir,
            );
        }
        AppServerEvent::UserMessageSubmitted { text } => {
            set_streamed_this_turn.set(false);
            set_active_stream_message_id.set(None);
            push_user_message_once(set_messages, next_message_id, set_next_message_id, text);
        }
        AppServerEvent::TurnOutput { output } => {
            set_is_sending.set(false);
            set_active_turn_id.set(None);
            set_agent_status.set("ожидает".to_owned());
            if streamed_this_turn.get() {
                if let Some(message_id) = active_stream_message_id.get() {
                    set_messages.update(|items| {
                        if let Some(message) =
                            items.iter_mut().find(|message| message.id == message_id)
                        {
                            message.streaming = false;
                        }
                    });
                } else {
                    finish_all_streaming_assistant_messages(set_messages);
                }
                set_active_stream_message_id.set(None);
                set_streamed_this_turn.set(false);
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
        AppServerEvent::Error { message } => {
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

fn update_runtime_status_and_tools(
    envelope: &Value,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    set_streamed_this_turn: WriteSignal<bool>,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
) {
    let Some(event) = envelope.get("event") else {
        return;
    };

    if event.get("TurnStarted").is_some() {
        set_streamed_this_turn.set(false);
        set_active_stream_message_id.set(None);
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
        set_agent_status.set("пишет".to_owned());
        if let Some(text) = delta_event.get("text").and_then(Value::as_str) {
            finish_streaming_reasoning(set_messages);
            set_streamed_this_turn.set(true);
            append_streaming_assistant_delta(
                set_messages,
                next_message_id,
                set_next_message_id,
                active_stream_message_id,
                set_active_stream_message_id,
                text,
            );
        }
    } else if let Some(reasoning_event) = event.get("AssistantReasoningDelta") {
        set_agent_status.set("думает".to_owned());
        if let Some(text) = reasoning_event.get("text").and_then(Value::as_str) {
            append_streaming_reasoning_delta(
                set_messages,
                next_message_id,
                set_next_message_id,
                text,
            );
        }
    } else if let Some(tool_event) = event.get("ToolCallRequested") {
        set_agent_status.set("запускает tool".to_owned());
        finish_active_streaming_assistant_message(
            set_messages,
            active_stream_message_id,
            set_active_stream_message_id,
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
            let args_preview = call.get("args").map(compact_json).unwrap_or_default();
            let tool = ToolActivity {
                call_id: call_id.clone(),
                name,
                args_preview,
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
            let preview = tool_result_preview(result);
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
        finish_streaming_reasoning(set_messages);
        set_agent_status.set("ожидает".to_owned());
    } else if event.get("Error").is_some() {
        set_agent_status.set("ошибка".to_owned());
    }
}

fn trim_tool_activities(items: &mut Vec<ToolActivity>) {
    let overflow = items.len().saturating_sub(12);
    if overflow > 0 {
        items.drain(0..overflow);
    }
}

fn tool_result_preview(result: &Value) -> String {
    if let Some(error) = result.get("error").and_then(Value::as_str)
        && !error.is_empty()
    {
        return compact_text(error, 600);
    }

    if let Some(output) = result.get("output").and_then(Value::as_str)
        && !output.is_empty()
    {
        return compact_text(output, 600);
    }

    if let Some(content) = result.get("content")
        && !content.as_array().is_none_or(Vec::is_empty)
    {
        return compact_text(&content.to_string(), 600);
    }

    if result.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        "(нет вывода)".to_owned()
    } else {
        "(tool завершился без текста ошибки)".to_owned()
    }
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
