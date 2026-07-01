use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{Value, json};
use web_sys::{EventSource, window};

use crate::actions::handle_command_response;
use crate::api::post_json;
use crate::app_helpers::{
    apply_active_session_activity, load_runtime_settings, load_sidebar_sessions, load_transcript,
    replace_transcript, replace_transcript_for_session,
};
use crate::events::{EventStreamBindings, close_event_stream, reconnect_event_stream};
use crate::messages::report_error;
use crate::types::*;
use crate::ui_utils::{short_id, short_path};

#[derive(Clone, Copy)]
pub(crate) struct RuntimeSettingsBindings {
    pub(crate) set_mode: WriteSignal<PermissionMode>,
    pub(crate) set_model_name: WriteSignal<String>,
    pub(crate) set_model_options: WriteSignal<Vec<String>>,
    pub(crate) set_reasoning_enabled: WriteSignal<bool>,
    pub(crate) set_effort: WriteSignal<ReasoningEffort>,
    pub(crate) set_effort_options: WriteSignal<Vec<String>>,
    pub(crate) set_workspace_label: WriteSignal<String>,
    pub(crate) set_active_session_dir: WriteSignal<Option<String>>,
    pub(crate) set_messages: WriteSignal<Vec<Message>>,
    pub(crate) next_message_id: ReadSignal<u64>,
    pub(crate) set_next_message_id: WriteSignal<u64>,
    pub(crate) set_transport_status: WriteSignal<TransportStatus>,
}

impl RuntimeSettingsBindings {
    pub(crate) fn load(self) {
        load_runtime_settings(
            self.set_mode,
            self.set_model_name,
            self.set_model_options,
            self.set_reasoning_enabled,
            self.set_effort,
            self.set_effort_options,
            self.set_workspace_label,
            self.set_active_session_dir,
            self.set_messages,
            self.next_message_id,
            self.set_next_message_id,
            self.set_transport_status,
        );
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TranscriptBindings {
    pub(crate) set_messages: WriteSignal<Vec<Message>>,
    pub(crate) transcript_generation: ReadSignal<u64>,
    pub(crate) next_message_id: ReadSignal<u64>,
    pub(crate) set_next_message_id: WriteSignal<u64>,
    pub(crate) set_transport_status: WriteSignal<TransportStatus>,
}

impl TranscriptBindings {
    pub(crate) fn load_initial(self, messages: ReadSignal<Vec<Message>>) {
        load_transcript(
            messages,
            self.set_messages,
            self.transcript_generation,
            self.transcript_generation.get_untracked(),
            self.next_message_id,
            self.set_next_message_id,
            self.set_transport_status,
        );
    }

    fn replace_current(self, expected_generation: u64) {
        replace_transcript(
            self.set_messages,
            self.transcript_generation,
            expected_generation,
            self.next_message_id,
            self.set_next_message_id,
            self.set_transport_status,
        );
    }

    fn replace_for_session(self, session_dir: String, expected_generation: u64) {
        replace_transcript_for_session(
            Some(session_dir),
            self.set_messages,
            self.transcript_generation,
            expected_generation,
            self.next_message_id,
            self.set_next_message_id,
            self.set_transport_status,
        );
    }
}

#[derive(Clone, Copy)]
pub(crate) struct AppSessionActions {
    pub(crate) event_source: StoredValue<Option<EventSource>, LocalStorage>,
    pub(crate) event_stream: EventStreamBindings,
    pub(crate) runtime_settings: RuntimeSettingsBindings,
    pub(crate) transcript: TranscriptBindings,
    pub(crate) active_session_dir: ReadSignal<Option<String>>,
    pub(crate) set_transcript_generation: WriteSignal<u64>,
    pub(crate) set_session_label: WriteSignal<String>,
    pub(crate) set_is_sending: WriteSignal<bool>,
    pub(crate) set_active_turn_id: WriteSignal<Option<String>>,
    pub(crate) set_active_stream_message_id: WriteSignal<Option<u64>>,
    pub(crate) set_streamed_this_turn: WriteSignal<bool>,
    pub(crate) set_agent_status: WriteSignal<String>,
    pub(crate) set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    pub(crate) set_queued_prompts: WriteSignal<Vec<(u64, String)>>,
    pub(crate) set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    pub(crate) set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
    pub(crate) set_stick_to_bottom: WriteSignal<bool>,
    pub(crate) set_sidebar_sessions: WriteSignal<Vec<SessionSummary>>,
    pub(crate) set_sidebar_sessions_status: WriteSignal<String>,
}

impl AppSessionActions {
    pub(crate) fn load_sidebar_sessions(self) {
        load_sidebar_sessions(self.set_sidebar_sessions, self.set_sidebar_sessions_status);
    }

    pub(crate) fn clear_transcript(self) {
        self.reset_chat_view();
        spawn_local(async move {
            match post_json("/clear", &json!({})).await {
                Ok(output) => handle_command_response(
                    output,
                    self.transcript.set_messages,
                    self.transcript.next_message_id,
                    self.transcript.set_next_message_id,
                    self.transcript.set_transport_status,
                ),
                Err(error) => {
                    report_error(
                        self.transcript.set_messages,
                        self.transcript.next_message_id,
                        self.transcript.set_next_message_id,
                        self.transcript.set_transport_status,
                        "Clear failed",
                        error,
                    );
                }
            }
            self.load_sidebar_sessions();
        });
    }

    pub(crate) fn start_new_session(self) {
        close_event_stream(self.event_source);
        self.reset_chat_view();
        let expected_generation = self.transcript.transcript_generation.get_untracked();
        self.runtime_settings.set_active_session_dir.set(None);
        self.set_session_label.set("not started".to_owned());
        self.set_sidebar_sessions_status
            .set("создаю новую сессию".to_owned());
        spawn_local(async move {
            match post_json("/new-session", &json!({ "id": "new-session" })).await {
                Ok(StdioOutput::Response { ok: true, .. }) => {
                    if self.transcript.transcript_generation.get_untracked() != expected_generation
                    {
                        return;
                    }
                    self.set_sidebar_sessions_status
                        .set("новая сессия открыта".to_owned());
                    reconnect_event_stream(self.event_source, self.event_stream);
                    self.runtime_settings.load();
                    self.transcript.replace_current(expected_generation);
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    self.set_sidebar_sessions_status
                        .set(error.unwrap_or_else(|| "не удалось создать сессию".to_owned()));
                }
                Ok(StdioOutput::Event { .. }) => {
                    self.set_sidebar_sessions_status
                        .set("неожиданное событие new-session".to_owned());
                }
                Err(error) => {
                    self.set_sidebar_sessions_status
                        .set(format!("не удалось создать сессию: {error}"));
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
        if let Some(workspace) = session.workspace_path.clone() {
            self.runtime_settings.set_workspace_label.set(workspace);
        }
        match session.session_id.clone() {
            Some(session_id) => self.set_session_label.set(short_id(&session_id).to_owned()),
            None => self.set_session_label.set(short_path(&session.session_dir)),
        }
        self.transcript.set_messages.set(Vec::new());
        self.transcript.set_next_message_id.set(1);
        self.set_queued_prompts.set(Vec::new());
        self.set_active_stream_message_id.set(None);
        self.set_streamed_this_turn.set(false);
        apply_active_session_activity(
            session.activity.as_ref(),
            self.set_is_sending,
            self.set_active_turn_id,
            self.set_agent_status,
        );
        self.set_tool_activities.set(Vec::new());
        self.set_pending_approvals.set(Vec::new());
        self.set_pending_user_inputs.set(Vec::new());

        let session_dir = session.session_dir.clone();
        self.transcript
            .replace_for_session(session_dir.clone(), expected_generation);
        self.set_sidebar_sessions_status
            .set("открываю сессию".to_owned());
        spawn_local(async move {
            match post_json(
                "/resume",
                &ResumeSessionRequest {
                    id: Some("sidebar-resume".to_owned()),
                    session_dir: session_dir.clone(),
                },
            )
            .await
            {
                Ok(StdioOutput::Response {
                    ok: true, output, ..
                }) => {
                    if self.transcript.transcript_generation.get_untracked() != expected_generation
                    {
                        return;
                    }
                    if let Some(activity) = output
                        .as_ref()
                        .and_then(|value| value.get("activity"))
                        .cloned()
                        .and_then(|value| serde_json::from_value::<SessionActivityInfo>(value).ok())
                    {
                        apply_active_session_activity(
                            Some(&activity),
                            self.set_is_sending,
                            self.set_active_turn_id,
                            self.set_agent_status,
                        );
                    }
                    self.set_sidebar_sessions_status
                        .set("сессия открыта".to_owned());
                    reconnect_event_stream(self.event_source, self.event_stream);
                    self.runtime_settings.load();
                    self.transcript
                        .replace_for_session(session_dir.clone(), expected_generation);
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    self.set_sidebar_sessions_status
                        .set(error.unwrap_or_else(|| "не удалось открыть сессию".to_owned()));
                }
                Ok(StdioOutput::Event { .. }) => {
                    self.set_sidebar_sessions_status
                        .set("неожиданное событие resume".to_owned());
                }
                Err(error) => {
                    self.set_sidebar_sessions_status
                        .set(format!("не удалось открыть сессию: {error}"));
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
                    self.set_sidebar_sessions.update(|items| {
                        items.retain(|item| item.session_dir != session_dir);
                    });
                    self.set_sidebar_sessions_status
                        .set("сессия удалена".to_owned());
                    let active_replaced = output
                        .as_ref()
                        .and_then(|value| value.get("active_replaced"))
                        .and_then(Value::as_bool)
                        .unwrap_or(deleting_active);
                    if active_replaced {
                        if self.transcript.transcript_generation.get_untracked()
                            != delete_request_generation
                        {
                            return;
                        }
                        self.reset_chat_view();
                        let expected_generation =
                            self.transcript.transcript_generation.get_untracked();
                        self.runtime_settings.set_active_session_dir.set(None);
                        self.set_session_label.set("not started".to_owned());
                        reconnect_event_stream(self.event_source, self.event_stream);
                        self.runtime_settings.load();
                        self.transcript.replace_current(expected_generation);
                    }
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    self.set_sidebar_sessions_status
                        .set(error.unwrap_or_else(|| "не удалось удалить сессию".to_owned()));
                }
                Ok(StdioOutput::Event { .. }) => {
                    self.set_sidebar_sessions_status
                        .set("неожиданное событие delete-session".to_owned());
                }
                Err(error) => {
                    self.set_sidebar_sessions_status
                        .set(format!("не удалось удалить сессию: {error}"));
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
        self.set_active_turn_id.set(None);
        self.set_agent_status.set("ожидает".to_owned());
        self.set_stick_to_bottom.set(true);
    }
}
