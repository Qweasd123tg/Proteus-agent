mod bootstrap;

use crate::events::EventConnection;
use leptos::prelude::*;
use leptos::task::spawn_local;
use web_sys::window;

use crate::api::{clear_selected_session_dir, persist_selected_session_dir, post_json};
use crate::events::{EventStreamBindings, close_event_stream, reconnect_event_stream};
use crate::session::history::{
    load_transcript, replace_transcript, replace_transcript_for_session,
};
use crate::session::settings::load_runtime_settings;
use crate::session::summaries::{apply_active_session_activity, load_sidebar_sessions};
use crate::types::*;
use crate::ui_preferences::{remove_context_usage, remove_session_draft};
use crate::ui_utils::short_id;
use bootstrap::create_session;

#[derive(Clone, Copy)]
pub(crate) struct RuntimeSettingsBindings {
    pub(crate) set_mode: WriteSignal<PermissionMode>,
    pub(crate) set_model_name: WriteSignal<String>,
    pub(crate) set_model_options: WriteSignal<Vec<ModelOption>>,
    pub(crate) set_reasoning_enabled: WriteSignal<bool>,
    pub(crate) set_effort: WriteSignal<ReasoningEffort>,
    pub(crate) set_effort_options: WriteSignal<Vec<String>>,
    pub(crate) set_workspace_label: WriteSignal<String>,
    pub(crate) set_active_session_dir: WriteSignal<Option<String>>,
    pub(crate) active_session_dir: ReadSignal<Option<String>>,
    pub(crate) transcript_generation: ReadSignal<u64>,
    pub(crate) set_is_sending: WriteSignal<bool>,
    pub(crate) set_active_run_id: WriteSignal<Option<String>>,
    pub(crate) set_agent_status: WriteSignal<String>,
    pub(crate) set_messages: crate::transcript::TranscriptWriter,
    pub(crate) next_message_id: ReadSignal<u64>,
    pub(crate) set_next_message_id: WriteSignal<u64>,
    pub(crate) set_transport_status: WriteSignal<TransportStatus>,
}

impl RuntimeSettingsBindings {
    pub(crate) fn load(self, session_dir: String, expected_generation: u64) {
        load_runtime_settings(
            session_dir,
            self.active_session_dir,
            self.transcript_generation,
            expected_generation,
            self.set_mode,
            self.set_model_name,
            self.set_model_options,
            self.set_reasoning_enabled,
            self.set_effort,
            self.set_effort_options,
            self.set_workspace_label,
            self.set_is_sending,
            self.set_active_run_id,
            self.set_agent_status,
            self.set_messages,
            self.next_message_id,
            self.set_next_message_id,
            self.set_transport_status,
        );
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TranscriptBindings {
    pub(crate) set_messages: crate::transcript::TranscriptWriter,
    pub(crate) transcript_generation: ReadSignal<u64>,
    pub(crate) next_message_id: ReadSignal<u64>,
    pub(crate) set_next_message_id: WriteSignal<u64>,
    pub(crate) set_active_stream_message_id: WriteSignal<Option<u64>>,
    pub(crate) set_streamed_this_turn: WriteSignal<bool>,
    pub(crate) set_transport_status: WriteSignal<TransportStatus>,
}

impl TranscriptBindings {
    pub(crate) fn load_initial(self, session_dir: String, messages: crate::transcript::Transcript) {
        load_transcript(
            session_dir,
            messages,
            self.set_messages,
            self.transcript_generation,
            self.transcript_generation.get_untracked(),
            self.next_message_id,
            self.set_next_message_id,
            self.set_active_stream_message_id,
            self.set_streamed_this_turn,
            self.set_transport_status,
        );
    }

    fn replace_current(self, session_dir: String, expected_generation: u64) {
        replace_transcript(
            session_dir,
            self.set_messages,
            self.transcript_generation,
            expected_generation,
            self.next_message_id,
            self.set_next_message_id,
            self.set_active_stream_message_id,
            self.set_streamed_this_turn,
            self.set_transport_status,
        );
    }

    fn replace_for_session(self, session_dir: String, expected_generation: u64) {
        replace_transcript_for_session(
            session_dir,
            self.set_messages,
            self.transcript_generation,
            expected_generation,
            self.next_message_id,
            self.set_next_message_id,
            self.set_active_stream_message_id,
            self.set_streamed_this_turn,
            self.set_transport_status,
        );
    }
}

#[derive(Clone, Copy)]
pub(crate) struct AppSessionActions {
    pub(crate) event_source: StoredValue<Option<EventConnection>, LocalStorage>,
    pub(crate) event_stream: EventStreamBindings,
    pub(crate) runtime_settings: RuntimeSettingsBindings,
    pub(crate) transcript: TranscriptBindings,
    pub(crate) active_session_dir: ReadSignal<Option<String>>,
    pub(crate) set_transcript_generation: WriteSignal<u64>,
    pub(crate) set_session_label: WriteSignal<String>,
    pub(crate) set_is_sending: WriteSignal<bool>,
    pub(crate) set_active_run_id: WriteSignal<Option<String>>,
    pub(crate) set_active_stream_message_id: WriteSignal<Option<u64>>,
    pub(crate) set_streamed_this_turn: WriteSignal<bool>,
    pub(crate) set_agent_status: WriteSignal<String>,
    pub(crate) set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    pub(crate) set_queued_prompts: WriteSignal<Vec<QueuedPromptInfo>>,
    pub(crate) set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    pub(crate) set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
    pub(crate) set_stick_to_bottom: WriteSignal<bool>,
    pub(crate) sidebar_sessions: ReadSignal<Vec<SessionSummary>>,
    pub(crate) set_sidebar_sessions: WriteSignal<Vec<SessionSummary>>,
    pub(crate) set_sidebar_sessions_status: WriteSignal<String>,
}

impl AppSessionActions {
    pub(crate) fn load_sidebar_sessions(self) {
        load_sidebar_sessions(self.set_sidebar_sessions, self.set_sidebar_sessions_status);
    }

    /// Переподключает event stream, если чат не переключился на другую
    /// generation, пока запрос был в полёте.
    fn reconnect_if_current(self, expected_generation: u64) {
        if self.transcript.transcript_generation.get_untracked() == expected_generation {
            reconnect_event_stream(self.event_source, self.event_stream);
        }
    }

    pub(crate) fn start_new_session(self) {
        let previous_session = self.active_session_dir.get_untracked();
        close_event_stream(self.event_source);
        self.reset_chat_view();
        let expected_generation = self.transcript.transcript_generation.get_untracked();
        self.runtime_settings.set_active_session_dir.set(None);
        self.set_session_label.set("not started".to_owned());
        self.set_sidebar_sessions_status
            .set("создаю новую сессию".to_owned());
        spawn_local(async move {
            let result = create_session(previous_session.clone()).await;
            if self.transcript.transcript_generation.get_untracked() != expected_generation {
                return;
            }
            match result {
                Ok(session_dir) => {
                    self.set_sidebar_sessions_status
                        .set("новая сессия открыта".to_owned());
                    self.activate_session(session_dir.clone());
                    reconnect_event_stream(self.event_source, self.event_stream);
                    self.runtime_settings
                        .load(session_dir.clone(), expected_generation);
                    self.transcript
                        .replace_current(session_dir, expected_generation);
                }
                Err(error) => {
                    self.set_sidebar_sessions_status
                        .set(format!("не удалось создать сессию: {error}"));
                    if let Some(previous_session) = previous_session {
                        self.activate_session(previous_session);
                    }
                    self.reconnect_if_current(expected_generation);
                }
            }
            self.load_sidebar_sessions();
        });
    }

    pub(crate) fn open_sidebar_session(self, session: SessionSummary) {
        if self.active_session_dir.get().as_deref() == Some(session.session_dir.as_str()) {
            return;
        }

        close_event_stream(self.event_source);
        let expected_generation = self.transcript.transcript_generation.get_untracked() + 1;
        self.set_transcript_generation.set(expected_generation);
        self.runtime_settings
            .set_active_session_dir
            .set(Some(session.session_dir.clone()));
        let _ = persist_selected_session_dir(&session.session_dir);
        self.runtime_settings
            .set_workspace_label
            .set(session.workspace_path.clone());
        self.set_session_label
            .set(short_id(&session.session_id).to_owned());
        self.transcript.set_messages.set(Vec::new());
        self.transcript.set_next_message_id.set(1);
        self.set_queued_prompts.set(Vec::new());
        self.set_active_stream_message_id.set(None);
        self.set_streamed_this_turn.set(false);
        apply_active_session_activity(
            session.activity.as_ref(),
            self.set_is_sending,
            self.set_active_run_id,
            self.set_agent_status,
        );
        self.set_tool_activities.set(Vec::new());
        self.set_pending_approvals.set(Vec::new());
        self.set_pending_user_inputs.set(Vec::new());

        let session_dir = session.session_dir.clone();
        self.set_sidebar_sessions_status
            .set("открываю сессию".to_owned());
        spawn_local(async move {
            let result = post_json(
                "/resume",
                &ResumeSessionRequest {
                    id: Some("sidebar-resume".to_owned()),
                    session_dir: session_dir.clone(),
                },
            )
            .await;
            if self.transcript.transcript_generation.get_untracked() != expected_generation
                || self.active_session_dir.get_untracked().as_deref() != Some(session_dir.as_str())
            {
                return;
            }
            match result {
                Ok(StdioOutput::Response {
                    ok: true, output, ..
                }) => {
                    if let Some(activity) = output
                        .as_ref()
                        .and_then(|value| value.get("activity"))
                        .cloned()
                        .and_then(|value| serde_json::from_value::<SessionActivityInfo>(value).ok())
                    {
                        apply_active_session_activity(
                            Some(&activity),
                            self.set_is_sending,
                            self.set_active_run_id,
                            self.set_agent_status,
                        );
                    }
                    self.set_sidebar_sessions_status
                        .set("сессия открыта".to_owned());
                    reconnect_event_stream(self.event_source, self.event_stream);
                    self.runtime_settings
                        .load(session_dir.clone(), expected_generation);
                    self.transcript
                        .replace_for_session(session_dir.clone(), expected_generation);
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    self.set_sidebar_sessions_status
                        .set(error.unwrap_or_else(|| "не удалось открыть сессию".to_owned()));
                    self.runtime_settings
                        .set_transport_status
                        .set(TransportStatus::Error(
                            "не удалось открыть выбранную сессию".to_owned(),
                        ));
                }
                Ok(StdioOutput::Event { .. }) => {
                    self.set_sidebar_sessions_status
                        .set("неожиданное событие resume".to_owned());
                    self.runtime_settings
                        .set_transport_status
                        .set(TransportStatus::Error(
                            "неожиданное событие resume".to_owned(),
                        ));
                }
                Err(error) => {
                    self.set_sidebar_sessions_status
                        .set(format!("не удалось открыть сессию: {error}"));
                    self.runtime_settings
                        .set_transport_status
                        .set(TransportStatus::Error(error));
                }
            }
            self.load_sidebar_sessions();
        });
    }

    pub(crate) fn delete_sidebar_session(self, session: SessionSummary) {
        let confirmed = window()
            .and_then(|window| window.confirm_with_message("Удалить этот чат?").ok())
            .unwrap_or(false);
        if !confirmed {
            return;
        }

        let session_dir = session.session_dir.clone();
        let deleting_active =
            self.active_session_dir.get().as_deref() == Some(session_dir.as_str());
        let delete_request_generation = self.transcript.transcript_generation.get_untracked();
        if deleting_active {
            close_event_stream(self.event_source);
        }
        self.set_sidebar_sessions_status
            .set("удаляю сессию".to_owned());
        spawn_local(async move {
            match post_json(
                "/delete-session",
                &DeleteSessionRequest {
                    id: Some("sidebar-delete".to_owned()),
                    session_dir: session_dir.clone(),
                },
            )
            .await
            {
                Ok(StdioOutput::Response {
                    ok: true, output, ..
                }) => {
                    let deleted = output
                        .as_ref()
                        .and_then(|value| value.get("deleted"))
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    if !deleted {
                        self.set_sidebar_sessions_status
                            .set("сервер не удалил сессию".to_owned());
                        if deleting_active {
                            self.reconnect_if_current(delete_request_generation);
                        }
                        return;
                    }
                    self.set_sidebar_sessions.update(|items| {
                        items.retain(|item| item.session_dir != session_dir);
                    });
                    remove_session_draft(&session_dir);
                    remove_context_usage(&session_dir);
                    self.set_sidebar_sessions_status
                        .set("сессия удалена".to_owned());
                    if deleting_active {
                        if self.transcript.transcript_generation.get_untracked()
                            != delete_request_generation
                        {
                            return;
                        }
                        self.reset_chat_view();
                        self.runtime_settings.set_active_session_dir.set(None);
                        let _ = clear_selected_session_dir();
                        self.set_session_label.set("not started".to_owned());
                        let replacement = self.sidebar_sessions_first();
                        if let Some(replacement) = replacement {
                            self.open_sidebar_session(replacement);
                        } else {
                            self.start_new_session();
                        }
                    }
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    self.set_sidebar_sessions_status
                        .set(error.unwrap_or_else(|| "не удалось удалить сессию".to_owned()));
                    if deleting_active {
                        self.reconnect_if_current(delete_request_generation);
                    }
                }
                Ok(StdioOutput::Event { .. }) => {
                    self.set_sidebar_sessions_status
                        .set("неожиданное событие delete-session".to_owned());
                    if deleting_active {
                        self.reconnect_if_current(delete_request_generation);
                    }
                }
                Err(error) => {
                    self.set_sidebar_sessions_status
                        .set(format!("не удалось удалить сессию: {error}"));
                    if deleting_active {
                        self.reconnect_if_current(delete_request_generation);
                    }
                }
            }
            self.load_sidebar_sessions();
        });
    }

    fn reset_chat_view(self) {
        self.set_transcript_generation
            .update(|generation| *generation += 1);
        self.transcript.set_messages.set(Vec::new());
        self.transcript.set_next_message_id.set(1);
        self.set_active_stream_message_id.set(None);
        self.set_streamed_this_turn.set(false);
        self.set_tool_activities.set(Vec::new());
        self.set_queued_prompts.set(Vec::new());
        self.set_pending_approvals.set(Vec::new());
        self.set_pending_user_inputs.set(Vec::new());
        self.set_is_sending.set(false);
        self.set_active_run_id.set(None);
        self.set_agent_status.set("ожидает".to_owned());
        self.set_stick_to_bottom.set(true);
    }

    fn activate_session(self, session_dir: String) {
        let _ = persist_selected_session_dir(&session_dir);
        self.runtime_settings
            .set_active_session_dir
            .set(Some(session_dir));
    }

    fn sidebar_sessions_first(self) -> Option<SessionSummary> {
        self.sidebar_sessions.get_untracked().first().cloned()
    }
}
