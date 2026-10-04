mod control;
mod preferences;
mod queue;
pub(crate) use control::ControlRequests;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::post_json;
use crate::messages::{push_message, push_user_message_once, report_error};
use crate::types::*;

#[derive(Clone, Copy)]
pub(crate) struct AppActions {
    pub(crate) controls: ControlRequests,
    pub(crate) event_count: ReadSignal<u64>,
    pub(crate) attachments: RwSignal<Vec<proteus_contracts::domain::ImageAttachment>>,
    pub(crate) attachments_loading: RwSignal<bool>,
    pub(crate) set_messages: crate::transcript::TranscriptWriter,
    pub(crate) next_message_id: ReadSignal<u64>,
    pub(crate) set_next_message_id: WriteSignal<u64>,
    pub(crate) set_transport_status: WriteSignal<TransportStatus>,
    pub(crate) active_session_dir: ReadSignal<Option<String>>,
    pub(crate) transcript_generation: ReadSignal<u64>,
    pub(crate) next_request_id: ReadSignal<u64>,
    pub(crate) set_next_request_id: WriteSignal<u64>,
    pub(crate) mode: ReadSignal<PermissionMode>,
    pub(crate) set_mode: WriteSignal<PermissionMode>,
    pub(crate) model_name: ReadSignal<String>,
    pub(crate) set_model_name: WriteSignal<String>,
    pub(crate) set_model_options: WriteSignal<Vec<ModelOption>>,
    pub(crate) set_effort_options: WriteSignal<Vec<String>>,
    pub(crate) set_reasoning_enabled: WriteSignal<bool>,
    pub(crate) effort: ReadSignal<ReasoningEffort>,
    pub(crate) set_effort: WriteSignal<ReasoningEffort>,
    pub(crate) is_sending: ReadSignal<bool>,
    pub(crate) set_is_sending: WriteSignal<bool>,
    pub(crate) active_run_id: ReadSignal<Option<String>>,
    pub(crate) set_active_run_id: WriteSignal<Option<String>>,
}

impl AppActions {
    pub(crate) fn send_prompt(
        self,
        text: String,
        intent: Option<&'static str>,
        forced_mode: Option<PermissionMode>,
    ) {
        let text = text.trim().to_owned();
        if self.attachments_loading.get_untracked()
            || (text.is_empty() && self.attachments.with_untracked(|images| images.is_empty()))
            || self.is_sending.get()
        {
            return;
        }
        let Some(session_dir) = self.active_session_dir.get_untracked() else {
            return;
        };

        if let Some(new_mode) = forced_mode {
            self.set_mode.set(new_mode);
        }

        self.set_is_sending.set(true);
        push_user_message_once(
            self.set_messages,
            self.next_message_id,
            self.set_next_message_id,
            text.clone(),
        );
        let request_id = take_request_id(self.next_request_id, self.set_next_request_id, "send");
        let run_id = request_id.clone();
        self.set_active_run_id.set(Some(run_id.clone()));

        let permission_mode = forced_mode.unwrap_or(self.mode.get_untracked());
        let generation = self.transcript_generation.get_untracked();
        let images = self.attachments.get_untracked();
        self.attachments.set(Vec::new());
        spawn_local(async move {
            match crate::api::post_json_for_admission(
                "/send-async",
                &SendRequest {
                    images,
                    id: Some(request_id),
                    text,
                    options: proteus_app_common::run_options::RunOptions {
                        intent: intent.map(str::to_owned),
                        permission_mode: Some(permission_mode),
                    },
                    session_dir: session_dir.clone().into(),
                },
            )
            .await
            {
                Ok(output) => {
                    if !self.is_active_run(&run_id) {
                        return;
                    }
                    if command_succeeded(&output) {
                        self.set_transport_status.set(TransportStatus::Connected);
                    } else {
                        self.finish_run();
                        handle_command_response(
                            output,
                            self.set_messages,
                            self.next_message_id,
                            self.set_next_message_id,
                            self.set_transport_status,
                        );
                    }
                }
                Err(error) => {
                    if !self.is_active_run(&run_id) {
                        return;
                    }
                    if error.rejected_before_admission {
                        self.finish_run();
                    } else {
                        // A lost response is ambiguous. Check authoritative activity, but
                        // never overwrite a newer subscription update with this snapshot.
                        let revision = self.event_count.get_untracked();
                        if let Ok(config) = crate::api::get_json::<
                            proteus_contracts::app_protocol::config::ConfigSummary,
                        >(&crate::api::session_path(
                            "/config",
                            &session_dir,
                        ))
                        .await
                        {
                            if self.is_active_run(&run_id)
                                && self.event_count.get_untracked() == revision
                            {
                                if let Some(activity) = config.activity.as_ref() {
                                    let state =
                                        crate::session::summaries::active_session_activity_state(
                                            Some(activity),
                                        );
                                    self.set_is_sending.set(state.is_sending);
                                    self.set_active_run_id.set(state.active_run_id);
                                }
                            }
                        }
                    }
                    if self.is_current_session(&session_dir, generation) {
                        self.push_error("Send failed", error.message);
                    }
                }
            }
        });
    }

    fn finish_run(self) {
        self.set_is_sending.set(false);
        self.set_active_run_id.set(None);
    }

    fn is_current_session(self, session_dir: &str, generation: u64) -> bool {
        self.transcript_generation.get_untracked() == generation
            && self.active_session_dir.get_untracked().as_deref() == Some(session_dir)
    }

    fn push_error(self, prefix: &str, error: String) {
        report_error(
            self.set_messages,
            self.next_message_id,
            self.set_next_message_id,
            self.set_transport_status,
            prefix,
            error,
        );
    }

    fn is_active_run(self, run_id: &str) -> bool {
        self.active_run_id.get().as_deref() == Some(run_id)
    }

    fn set_control_error(self, prefix: &str, error: String) {
        self.set_transport_status
            .set(TransportStatus::Error(format!("{prefix}: {error}")));
    }
}

pub(crate) fn handle_command_response(
    output: StdioOutput,
    set_messages: crate::transcript::TranscriptWriter,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    if let StdioOutput::Response {
        id,
        ok,
        output: _,
        error,
    } = output
    {
        if ok {
            // Ответ дошёл — транспорт жив; ошибка прошлой команды не должна
            // оставлять бейдж в состоянии "ошибка" навсегда.
            set_transport_status.set(TransportStatus::Connected);
        } else {
            let message = error.unwrap_or_else(|| "request failed".to_owned());
            set_transport_status.set(TransportStatus::Error(message.clone()));
            push_message(
                set_messages,
                next_message_id,
                set_next_message_id,
                MessageRole::System,
                format!(
                    "{} failed: {message}",
                    id.unwrap_or_else(|| "request".to_owned())
                ),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cancel_active_run(
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
    let Some(target_id) = active_run_id.get() else {
        return;
    };
    let Some(session_dir) = active_session_dir.get_untracked() else {
        return;
    };
    let generation = transcript_generation.get_untracked();
    let request_id = take_request_id(next_request_id, set_next_request_id, "cancel");
    spawn_local(async move {
        match post_json(
            &crate::api::session_path("/cancel", &session_dir),
            &CancelRequest {
                id: Some(request_id),
                target_id,
            },
        )
        .await
        {
            Ok(output) => {
                if transcript_generation.get_untracked() != generation
                    || active_session_dir.get_untracked().as_deref() != Some(session_dir.as_str())
                {
                    return;
                }
                handle_command_response(
                    output,
                    set_messages,
                    next_message_id,
                    set_next_message_id,
                    set_transport_status,
                );
            }
            Err(error) => {
                if transcript_generation.get_untracked() != generation
                    || active_session_dir.get_untracked().as_deref() != Some(session_dir.as_str())
                {
                    return;
                }
                report_error(
                    set_messages,
                    next_message_id,
                    set_next_message_id,
                    set_transport_status,
                    "Cancel failed",
                    error,
                );
            }
        }
    });
}

pub(crate) fn send_prompt_for_mode(actions: AppActions, mode: PermissionMode, text: String) {
    if mode == PermissionMode::Plan {
        send_planning_request(actions, text);
    } else {
        actions.send_prompt(text, None, None);
    }
}

pub(crate) fn send_planning_request(actions: AppActions, text: String) {
    actions.send_prompt(text, Some("planning.start"), Some(PermissionMode::Plan));
}

/// Суффикс поколения загрузки страницы. Id send-запросов служат transport
/// run ids на сервере, а счётчик живёт в памяти приложения: после перезагрузки
/// он снова начинается с 1, и без уникального суффикса новый «send-1»
/// сталкивается с ещё выполняющимся run прошлой загрузки.
fn boot_nonce() -> &'static str {
    static NONCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NONCE.get_or_init(|| {
        #[cfg(target_arch = "wasm32")]
        {
            let ms = js_sys::Date::now().max(0.0) as u64;
            let salt = (js_sys::Math::random() * f64::from(u16::MAX)) as u16;
            format!("{:x}{salt:x}", ms & 0xffff_ffff)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            "boot".to_owned()
        }
    })
}

pub(crate) fn take_request_id(
    next_request_id: ReadSignal<u64>,
    set_next_request_id: WriteSignal<u64>,
    prefix: &str,
) -> String {
    let id = next_request_id.get();
    set_next_request_id.set(id + 1);
    format!("{prefix}-{}-{id}", boot_nonce())
}

fn command_succeeded(output: &StdioOutput) -> bool {
    matches!(output, StdioOutput::Response { ok: true, .. })
}
