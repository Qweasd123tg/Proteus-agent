mod connection;
mod control_plane;
pub(crate) use connection::{EventConnection, close_event_stream, reconnect_event_stream};
use control_plane::PendingControlPlane;
mod queue;
mod runtime;
mod stream;

use leptos::prelude::*;
use wasm_bindgen::JsValue;

use self::runtime::{
    event_updates_visible_count, update_runtime_status_and_tools, update_session_labels,
};
pub(crate) use self::stream::BufferedStreamDeltas;
use self::stream::{StreamFlushBindings, flush_stream_delta_buffer, set_stream_turn_thread};
use crate::actions::handle_command_response;
use crate::messages::{finalize_running_activity, push_message, push_user_message_once};
use crate::session::history::apply_transcript;
use crate::session::summaries::load_sidebar_sessions;
use crate::types::*;

#[derive(Clone, Copy)]
pub(crate) struct EventStreamBindings {
    pub(crate) set_messages: crate::transcript::TranscriptWriter,
    pub(crate) next_message_id: ReadSignal<u64>,
    pub(crate) set_next_message_id: WriteSignal<u64>,
    pub(crate) transport_status: ReadSignal<TransportStatus>,
    pub(crate) set_transport_status: WriteSignal<TransportStatus>,
    pub(crate) set_event_count: WriteSignal<u64>,
    pub(crate) set_workspace_label: WriteSignal<String>,
    pub(crate) set_session_label: WriteSignal<String>,
    pub(crate) active_session_dir: ReadSignal<Option<String>>,
    pub(crate) set_is_sending: WriteSignal<bool>,
    pub(crate) set_active_run_id: WriteSignal<Option<String>>,
    pub(crate) set_plan_run_id: WriteSignal<Option<String>>,
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
    pub(crate) set_queued_prompts: WriteSignal<Vec<QueuedPromptInfo>>,
    pub(crate) set_sidebar_sessions: WriteSignal<Vec<SessionSummary>>,
    pub(crate) set_sidebar_sessions_status: WriteSignal<String>,
}

#[allow(clippy::too_many_arguments)]
fn handle_app_output(
    output: StdioOutput,
    set_messages: crate::transcript::TranscriptWriter,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
    set_event_count: WriteSignal<u64>,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
    active_session_dir: ReadSignal<Option<String>>,
    set_is_sending: WriteSignal<bool>,
    set_active_run_id: WriteSignal<Option<String>>,
    set_plan_run_id: WriteSignal<Option<String>>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    streamed_this_turn: ReadSignal<bool>,
    set_streamed_this_turn: WriteSignal<bool>,
    stream_delta_buffer: StoredValue<BufferedStreamDeltas, LocalStorage>,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_context_usage: WriteSignal<Option<ContextUsage>>,
    pending: &PendingControlPlane,
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
                set_is_sending,
                set_active_run_id,
                set_plan_run_id,
                active_stream_message_id,
                set_active_stream_message_id,
                streamed_this_turn,
                set_streamed_this_turn,
                stream_delta_buffer,
                set_agent_status,
                set_tool_activities,
                set_context_usage,
                pending,
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
    set_messages: crate::transcript::TranscriptWriter,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
    set_workspace_label: WriteSignal<String>,
    set_session_label: WriteSignal<String>,
    active_session_dir: ReadSignal<Option<String>>,
    set_is_sending: WriteSignal<bool>,
    set_active_run_id: WriteSignal<Option<String>>,
    set_plan_run_id: WriteSignal<Option<String>>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    streamed_this_turn: ReadSignal<bool>,
    set_streamed_this_turn: WriteSignal<bool>,
    stream_delta_buffer: StoredValue<BufferedStreamDeltas, LocalStorage>,
    set_agent_status: WriteSignal<String>,
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_context_usage: WriteSignal<Option<ContextUsage>>,
    pending: &PendingControlPlane,
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
                active_session_dir,
                set_context_usage,
            );
            update_session_labels(envelope, set_workspace_label, set_session_label);
        }
        AppServerEvent::UserMessageSubmitted { text } => {
            flush_stream_delta_buffer(stream_bindings);
            set_streamed_this_turn.set(false);
            set_active_stream_message_id.set(None);
            push_user_message_once(set_messages, next_message_id, set_next_message_id, text);
        }
        AppServerEvent::SessionSnapshot { snapshot } => {
            let _identity = (&snapshot.session_id, &snapshot.stream_id, snapshot.seq);
            stream_delta_buffer.set_value(BufferedStreamDeltas::default());
            set_stream_turn_thread(stream_bindings, snapshot.root_thread_id.as_deref());
            set_tool_activities.set(Vec::new());
            apply_transcript(
                snapshot.transcript,
                set_messages,
                set_next_message_id,
                set_active_stream_message_id,
                set_streamed_this_turn,
            );
            apply_execution(
                snapshot.execution,
                set_is_sending,
                set_active_run_id,
                set_plan_run_id,
                set_agent_status,
            );
        }
        AppServerEvent::ExecutionUpdated { execution } => {
            apply_execution(
                execution,
                set_is_sending,
                set_active_run_id,
                set_plan_run_id,
                set_agent_status,
            );
        }
        AppServerEvent::TurnOutput { output } => {
            let _ = output;
            load_sidebar_sessions(set_sidebar_sessions, set_sidebar_sessions_status);
        }
        AppServerEvent::PendingRequestsUpdated { snapshot } => pending.apply_stream(*snapshot),
        // These remain occurrence notifications. Only a versioned snapshot
        // changes the local replica of pending state.
        AppServerEvent::ApprovalRequested { request } => {
            let _ = request;
        }
        AppServerEvent::ApprovalResolved {
            approval_id,
            approved,
        } => {
            let _ = (approval_id, approved);
        }
        AppServerEvent::UserInputRequested { request } => {
            let _ = request;
        }
        AppServerEvent::UserInputResolved { request_id } => {
            let _ = request_id;
        }
        AppServerEvent::ModulesReloaded {
            old_epoch,
            new_epoch,
            tool_names,
        } => {
            set_agent_status.set(format!(
                "модули обновлены: epoch {old_epoch} → {new_epoch}, tools {}",
                tool_names.len()
            ));
        }
        AppServerEvent::SessionActivityUpdated {
            session_dir,
            activity,
        } => {
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
            // Terminal transcript and execution arrive in the server snapshot.
            web_sys::console::warn_1(&JsValue::from_str(&message));
        }
        AppServerEvent::EventStreamLagged { count } => {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "event stream lagged by {count}; waiting for session snapshot"
            )));
            stream_delta_buffer.set_value(BufferedStreamDeltas::default());
            pending.refresh();
        }
        AppServerEvent::Shutdown => {
            flush_stream_delta_buffer(stream_bindings);
            set_is_sending.set(false);
            set_active_run_id.set(None);
            set_agent_status.set("остановлено".to_owned());
            set_transport_status.set(TransportStatus::Shutdown);
            finalize_running_activity(set_tool_activities, set_messages, crate::ui_utils::now_ms());
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                "AppServer shutdown".to_owned(),
            );
        }
    }
}

fn apply_execution(
    execution: proteus_client_common::execution::ExecutionState,
    set_is_sending: WriteSignal<bool>,
    set_active_run_id: WriteSignal<Option<String>>,
    set_plan_run_id: WriteSignal<Option<String>>,
    set_agent_status: WriteSignal<String>,
) {
    use proteus_client_common::execution::RunStatus;
    set_plan_run_id.set(
        execution
            .last
            .as_ref()
            .filter(|r| {
                r.status == RunStatus::Success
                    && matches!(
                        r.options.intent.as_deref(),
                        Some("planning.start" | "planning.revise")
                    )
            })
            .map(|r| r.run_id.clone()),
    );
    set_is_sending.set(execution.active.is_some());
    set_active_run_id.set(execution.active.as_ref().map(|r| r.run_id.clone()));
    let status = match execution
        .active
        .as_ref()
        .or(execution.last.as_ref())
        .map(|r| r.status)
    {
        Some(RunStatus::CancelRequested) => "отменяется",
        Some(RunStatus::Running) => "работает",
        Some(RunStatus::Canceled) => "отменено",
        Some(RunStatus::Timeout) => "таймаут",
        Some(RunStatus::Error) => "ошибка",
        _ => "ожидает",
    };
    set_agent_status.update(|current| {
        if status == "работает" && (current == "ждёт доступ" || current == "ждёт ответ")
        {
            return;
        }
        *current = status.to_owned();
    });
}
