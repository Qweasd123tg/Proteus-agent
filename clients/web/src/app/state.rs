use crate::{app_resize::AppResizeState, events::BufferedStreamDeltas, types::*};
use leptos::{html, prelude::*};

#[derive(Clone, Copy)]
pub(super) struct AppState {
    pub chat: ChatState,
    pub request: RequestState,
    pub session: SessionState,
    pub view: ViewState,
    pub user_messages: Memo<Vec<(u64, String)>>,
}
impl AppState {
    pub fn new() -> Self {
        let chat = ChatState::new();
        let messages = chat.messages;
        let user_messages = Memo::new(move |_| {
            messages.with(|items| {
                items
                    .iter()
                    .filter(|m| m.role == MessageRole::User)
                    .map(|m| (m.id, crate::ui_utils::compact_text(m.text.trim(), 80)))
                    .collect()
            })
        });
        Self {
            chat,
            request: RequestState::new(),
            session: SessionState::new(),
            view: ViewState::new(),
            user_messages,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ChatState {
    pub next_message_id: ReadSignal<u64>,
    pub set_next_message_id: WriteSignal<u64>,
    pub is_sending: ReadSignal<bool>,
    pub set_is_sending: WriteSignal<bool>,
    pub active_run_id: ReadSignal<Option<String>>,
    pub set_active_run_id: WriteSignal<Option<String>>,
    pub active_stream_message_id: ReadSignal<Option<u64>>,
    pub set_active_stream_message_id: WriteSignal<Option<u64>>,
    pub streamed_this_turn: ReadSignal<bool>,
    pub set_streamed_this_turn: WriteSignal<bool>,
    pub agent_status: ReadSignal<String>,
    pub set_agent_status: WriteSignal<String>,
    pub tool_activities: ReadSignal<Vec<ToolActivity>>,
    pub set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    pub transcript_generation: ReadSignal<u64>,
    pub set_transcript_generation: WriteSignal<u64>,
    pub pending_approvals: ReadSignal<Vec<ApprovalRequestInfo>>,
    pub set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    pub pending_user_inputs: ReadSignal<Vec<UserInputRequestInfo>>,
    pub set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
    pub messages: crate::transcript::Transcript,
    pub set_messages: crate::transcript::TranscriptWriter,
    pub stream_delta_buffer: StoredValue<BufferedStreamDeltas, LocalStorage>,
}
impl ChatState {
    fn new() -> Self {
        let (messages, set_messages) = crate::transcript::transcript(Vec::new());
        let (next_message_id, set_next_message_id) = signal(1_u64);
        let (is_sending, set_is_sending) = signal(false);
        let (active_run_id, set_active_run_id) = signal(None);
        let (active_stream_message_id, set_active_stream_message_id) = signal(None);
        let (streamed_this_turn, set_streamed_this_turn) = signal(false);
        let (agent_status, set_agent_status) = signal("ожидает".to_owned());
        let (tool_activities, set_tool_activities) = signal(Vec::new());
        let (transcript_generation, set_transcript_generation) = signal(0);
        let (pending_approvals, set_pending_approvals) = signal(Vec::new());
        let (pending_user_inputs, set_pending_user_inputs) = signal(Vec::new());
        let stream_delta_buffer = StoredValue::new_local(BufferedStreamDeltas::default());
        Self {
            next_message_id,
            set_next_message_id,
            is_sending,
            set_is_sending,
            active_run_id,
            set_active_run_id,
            active_stream_message_id,
            set_active_stream_message_id,
            streamed_this_turn,
            set_streamed_this_turn,
            agent_status,
            set_agent_status,
            tool_activities,
            set_tool_activities,
            transcript_generation,
            set_transcript_generation,
            pending_approvals,
            set_pending_approvals,
            pending_user_inputs,
            set_pending_user_inputs,
            messages,
            set_messages,
            stream_delta_buffer,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct RequestState {
    pub draft: ReadSignal<String>,
    pub set_draft: WriteSignal<String>,
    pub queued_prompts: ReadSignal<Vec<QueuedPromptInfo>>,
    pub set_queued_prompts: WriteSignal<Vec<QueuedPromptInfo>>,
    pub mode: ReadSignal<PermissionMode>,
    pub set_mode: WriteSignal<PermissionMode>,
    pub model_name: ReadSignal<String>,
    pub set_model_name: WriteSignal<String>,
    pub model_options: ReadSignal<Vec<ModelOption>>,
    pub set_model_options: WriteSignal<Vec<ModelOption>>,
    pub reasoning_enabled: ReadSignal<bool>,
    pub set_reasoning_enabled: WriteSignal<bool>,
    pub effort: ReadSignal<ReasoningEffort>,
    pub set_effort: WriteSignal<ReasoningEffort>,
    pub effort_options: ReadSignal<Vec<String>>,
    pub set_effort_options: WriteSignal<Vec<String>>,
    pub next_request_id: ReadSignal<u64>,
    pub set_next_request_id: WriteSignal<u64>,
}
impl RequestState {
    fn new() -> Self {
        let (draft, set_draft) = signal(String::new());
        let (queued_prompts, set_queued_prompts) = signal(Vec::new());
        let (mode, set_mode) = signal(PermissionMode::Normal);
        let (model_name, set_model_name) = signal(String::new());
        let (model_options, set_model_options) = signal(Vec::new());
        let (reasoning_enabled, set_reasoning_enabled) = signal(true);
        let (effort, set_effort) = signal(ReasoningEffort::Config);
        let (effort_options, set_effort_options) = signal(Vec::new());
        let (next_request_id, set_next_request_id) = signal(1);
        Self {
            draft,
            set_draft,
            queued_prompts,
            set_queued_prompts,
            mode,
            set_mode,
            model_name,
            set_model_name,
            model_options,
            set_model_options,
            reasoning_enabled,
            set_reasoning_enabled,
            effort,
            set_effort,
            effort_options,
            set_effort_options,
            next_request_id,
            set_next_request_id,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct SessionState {
    pub transport_status: ReadSignal<TransportStatus>,
    pub set_transport_status: WriteSignal<TransportStatus>,
    pub event_count: ReadSignal<u64>,
    pub set_event_count: WriteSignal<u64>,
    pub workspace_label: ReadSignal<String>,
    pub set_workspace_label: WriteSignal<String>,
    pub _session_label: ReadSignal<String>,
    pub set_session_label: WriteSignal<String>,
    pub active_session_dir: ReadSignal<Option<String>>,
    pub set_active_session_dir: WriteSignal<Option<String>>,
    pub context_usage: ReadSignal<Option<ContextUsage>>,
    pub set_context_usage: WriteSignal<Option<ContextUsage>>,
    pub sidebar_sessions: ReadSignal<Vec<SessionSummary>>,
    pub set_sidebar_sessions: WriteSignal<Vec<SessionSummary>>,
    pub sidebar_sessions_status: ReadSignal<String>,
    pub set_sidebar_sessions_status: WriteSignal<String>,
}
impl SessionState {
    fn new() -> Self {
        let (transport_status, set_transport_status) = signal(TransportStatus::Connecting);
        let (event_count, set_event_count) = signal(0);
        let (workspace_label, set_workspace_label) = signal("waiting for session".to_owned());
        let (_session_label, set_session_label) = signal("not started".to_owned());
        let (active_session_dir, set_active_session_dir) = signal(None);
        let (context_usage, set_context_usage) = signal(None);
        let (sidebar_sessions, set_sidebar_sessions) = signal(Vec::new());
        let (sidebar_sessions_status, set_sidebar_sessions_status) =
            signal("сессии не загружены".to_owned());
        Self {
            transport_status,
            set_transport_status,
            event_count,
            set_event_count,
            workspace_label,
            set_workspace_label,
            _session_label,
            set_session_label,
            active_session_dir,
            set_active_session_dir,
            context_usage,
            set_context_usage,
            sidebar_sessions,
            set_sidebar_sessions,
            sidebar_sessions_status,
            set_sidebar_sessions_status,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ViewState {
    pub toasts: ReadSignal<Vec<ToastMessage>>,
    pub set_toasts: WriteSignal<Vec<ToastMessage>>,
    pub next_toast_id: ReadSignal<u64>,
    pub set_next_toast_id: WriteSignal<u64>,
    pub last_error_toast: ReadSignal<Option<String>>,
    pub set_last_error_toast: WriteSignal<Option<String>>,
    pub stick_to_bottom: ReadSignal<bool>,
    pub set_stick_to_bottom: WriteSignal<bool>,
    pub scroll_frame_pending: ReadSignal<bool>,
    pub set_scroll_frame_pending: WriteSignal<bool>,
    pub last_results_scroll_top: ReadSignal<i32>,
    pub set_last_results_scroll_top: WriteSignal<i32>,
    pub active_user_message: ReadSignal<Option<u64>>,
    pub set_active_user_message: WriteSignal<Option<u64>>,
    pub tool_cards_collapsed: ReadSignal<bool>,
    pub set_tool_cards_collapsed: WriteSignal<bool>,
    pub activity_now_ms: ReadSignal<u64>,
    pub set_activity_now_ms: WriteSignal<u64>,
    pub detach_baseline: ReadSignal<Option<usize>>,
    pub set_detach_baseline: WriteSignal<Option<usize>>,
    pub results_ref: NodeRef<html::Section>,
    pub composer_ref: NodeRef<html::Textarea>,
    pub resize: AppResizeState,
}
impl ViewState {
    fn new() -> Self {
        let (toasts, set_toasts) = signal(Vec::new());
        let (next_toast_id, set_next_toast_id) = signal(1);
        let (last_error_toast, set_last_error_toast) = signal(None);
        let (stick_to_bottom, set_stick_to_bottom) = signal(true);
        let (scroll_frame_pending, set_scroll_frame_pending) = signal(false);
        let (last_results_scroll_top, set_last_results_scroll_top) = signal(0);
        let (active_user_message, set_active_user_message) = signal(None);
        let (tool_cards_collapsed, set_tool_cards_collapsed) = signal(false);
        let (activity_now_ms, set_activity_now_ms) = signal(js_sys::Date::now().max(0.0) as u64);
        let (detach_baseline, set_detach_baseline) = signal(None);
        let results_ref = NodeRef::new();
        let composer_ref = NodeRef::new();
        let resize = AppResizeState::new();
        Self {
            toasts,
            set_toasts,
            next_toast_id,
            set_next_toast_id,
            last_error_toast,
            set_last_error_toast,
            stick_to_bottom,
            set_stick_to_bottom,
            scroll_frame_pending,
            set_scroll_frame_pending,
            last_results_scroll_top,
            set_last_results_scroll_top,
            active_user_message,
            set_active_user_message,
            tool_cards_collapsed,
            set_tool_cards_collapsed,
            activity_now_ms,
            set_activity_now_ms,
            detach_baseline,
            set_detach_baseline,
            results_ref,
            composer_ref,
            resize,
        }
    }
}
