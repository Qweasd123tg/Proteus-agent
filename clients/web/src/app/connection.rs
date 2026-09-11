use super::state::AppState;
use crate::events::EventConnection;
use crate::{
    actions::AppActions,
    api::load_session_token,
    app_sessions::{AppSessionActions, RuntimeSettingsBindings, TranscriptBindings},
    events::{EventStreamBindings, reconnect_event_stream},
    types::*,
};
use leptos::prelude::*;
#[derive(Clone, Copy)]
pub(super) struct ClientConnection {
    pub actions: AppActions,
    pub session_actions: AppSessionActions,
    pub event_source: StoredValue<Option<EventConnection>, LocalStorage>,
    pub event_stream: EventStreamBindings,
}
impl ClientConnection {
    pub fn reconnect(self) {
        reconnect_event_stream(self.event_source, self.event_stream);
    }
}
pub(super) fn connect(state: AppState) -> ClientConnection {
    let super::state::ChatState {
        next_message_id,
        set_next_message_id,
        is_sending,
        set_is_sending,
        active_run_id,
        set_active_run_id,
        set_plan_run_id,
        active_stream_message_id,
        set_active_stream_message_id,
        streamed_this_turn,
        set_streamed_this_turn,
        set_agent_status,
        set_tool_activities,
        transcript_generation,
        set_transcript_generation,
        set_pending_approvals,
        set_pending_user_inputs,
        set_messages,
        stream_delta_buffer,
        ..
    } = state.chat;
    let super::state::RequestState {
        set_queued_prompts,
        mode,
        set_mode,
        model_name,
        set_model_name,
        set_model_options,
        reasoning_enabled,
        set_reasoning_enabled,
        effort,
        set_effort,
        set_effort_options,
        next_request_id,
        set_next_request_id,
        ..
    } = state.request;
    let super::state::SessionState {
        transport_status,
        set_transport_status,
        set_event_count,
        set_workspace_label,
        set_session_label,
        active_session_dir,
        set_active_session_dir,
        set_context_usage,
        set_sidebar_sessions,
        set_sidebar_sessions_status,
        ..
    } = state.session;
    let super::state::ViewState {
        set_stick_to_bottom,
        ..
    } = state.view;
    let _session_token = match load_session_token() {
        Ok(token) => token,
        Err(error) => {
            let message = format!("Session token storage failed: {error}");
            set_messages.set(vec![Message {
                message_id: None,
                phase: None,
                id: 1,
                version: 0,
                text_offset: 0,
                role: MessageRole::System,
                text: message,
                tool: None,
                subagent: None,
                streaming: false,
            }]);
            set_next_message_id.set(2);
            SessionToken::missing()
        }
    };
    let runtime_settings = RuntimeSettingsBindings {
        set_mode,
        set_model_name,
        set_model_options,
        set_reasoning_enabled,
        set_effort,
        set_effort_options,
        set_workspace_label,
        set_active_session_dir,
        active_session_dir,
        transcript_generation,
        set_messages,
        next_message_id,
        set_next_message_id,
        set_transport_status,
    };
    let transcript_bindings = TranscriptBindings {
        set_messages,
        transcript_generation,
        set_next_message_id,
    };

    let event_source = StoredValue::new_local(None::<EventConnection>);
    let event_stream_bindings = EventStreamBindings {
        set_messages,
        next_message_id,
        set_next_message_id,
        transport_status,
        set_transport_status,
        set_event_count,
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
        transcript_generation,
        set_pending_approvals,
        set_pending_user_inputs,
        set_queued_prompts,
        set_sidebar_sessions,
        set_sidebar_sessions_status,
    };
    let session_actions = AppSessionActions {
        event_source,
        event_stream: event_stream_bindings,
        runtime_settings,
        transcript: transcript_bindings,
        active_session_dir,
        set_transcript_generation,
        set_session_label,
        set_is_sending,
        set_active_run_id,
        set_active_stream_message_id,
        set_streamed_this_turn,
        set_agent_status,
        set_tool_activities,
        set_queued_prompts,
        set_pending_approvals,
        set_pending_user_inputs,
        set_stick_to_bottom,
        set_sidebar_sessions,
        sidebar_sessions: state.session.sidebar_sessions,
        set_sidebar_sessions_status,
    };
    session_actions.initialize();
    let actions = AppActions {
        set_messages,
        next_message_id,
        set_next_message_id,
        set_transport_status,
        active_session_dir,
        transcript_generation,
        next_request_id,
        set_next_request_id,
        mode,
        set_mode,
        model_name,
        set_model_name,
        set_model_options,
        set_effort_options,
        reasoning_enabled,
        set_reasoning_enabled,
        effort,
        set_effort,
        is_sending,
        set_is_sending,
        active_run_id,
        set_active_run_id,
    };

    on_cleanup(move || crate::events::close_event_stream(event_source));
    ClientConnection {
        actions,
        session_actions,
        event_source,
        event_stream: event_stream_bindings,
    }
}
