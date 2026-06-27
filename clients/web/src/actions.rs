use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::post_json;
use crate::messages::{push_message, push_user_message_once, report_error};
use crate::types::*;

#[derive(Clone, Copy)]
pub(crate) struct AppActions {
    pub(crate) set_messages: WriteSignal<Vec<Message>>,
    pub(crate) next_message_id: ReadSignal<u64>,
    pub(crate) set_next_message_id: WriteSignal<u64>,
    pub(crate) set_transport_status: WriteSignal<TransportStatus>,
    pub(crate) active_session_dir: ReadSignal<Option<String>>,
    pub(crate) next_request_id: ReadSignal<u64>,
    pub(crate) set_next_request_id: WriteSignal<u64>,
    pub(crate) mode: ReadSignal<PermissionMode>,
    pub(crate) set_mode: WriteSignal<PermissionMode>,
    pub(crate) model_name: ReadSignal<String>,
    pub(crate) set_model_name: WriteSignal<String>,
    pub(crate) reasoning_enabled: ReadSignal<bool>,
    pub(crate) set_reasoning_enabled: WriteSignal<bool>,
    pub(crate) effort: ReadSignal<ReasoningEffort>,
    pub(crate) set_effort: WriteSignal<ReasoningEffort>,
    pub(crate) is_sending: ReadSignal<bool>,
    pub(crate) set_is_sending: WriteSignal<bool>,
    pub(crate) active_turn_id: ReadSignal<Option<String>>,
    pub(crate) set_active_turn_id: WriteSignal<Option<String>>,
}

impl AppActions {
    pub(crate) fn set_permission_mode(self, new_mode: PermissionMode) {
        let previous_mode = self.mode.get();
        let session_dir = self.active_session_dir.get_untracked();
        self.set_mode.set(new_mode);
        let request_id = take_request_id(self.next_request_id, self.set_next_request_id, "mode");
        spawn_local(async move {
            match post_json(
                "/mode",
                &SetPermissionModeRequest {
                    id: Some(request_id),
                    mode: new_mode,
                    session_dir,
                },
            )
            .await
            {
                Ok(output) => {
                    if !handle_control_response(
                        output,
                        self.set_transport_status,
                        "Mode update failed",
                    ) {
                        self.set_mode.set(previous_mode);
                    }
                }
                Err(error) => {
                    self.set_mode.set(previous_mode);
                    self.set_control_error("Mode update failed", error);
                }
            }
        });
    }

    pub(crate) fn set_model_name(self, new_model: String) {
        let new_model = new_model.trim().to_owned();
        if new_model.is_empty() {
            return;
        }
        if self.model_name.get() == new_model {
            return;
        }
        let request_id = take_request_id(self.next_request_id, self.set_next_request_id, "model");
        let session_dir = self.active_session_dir.get_untracked();
        spawn_local(async move {
            let requested_model = new_model.clone();
            match post_json(
                "/model",
                &SetModelRequest {
                    id: Some(request_id),
                    model: requested_model,
                    session_dir,
                },
            )
            .await
            {
                Ok(output) => {
                    if handle_control_response(
                        output,
                        self.set_transport_status,
                        "Model update failed",
                    ) {
                        self.set_model_name.set(new_model);
                    }
                }
                Err(error) => self.set_control_error("Model update failed", error),
            }
        });
    }

    pub(crate) fn set_reasoning_enabled(self, enabled: bool) {
        let previous_enabled = self.reasoning_enabled.get();
        let previous_effort = self.effort.get();
        if previous_enabled == enabled {
            return;
        }
        self.set_reasoning_enabled.set(enabled);
        if !enabled {
            self.set_effort.set(ReasoningEffort::Config);
        }
        let request_id =
            take_request_id(self.next_request_id, self.set_next_request_id, "reasoning");
        let session_dir = self.active_session_dir.get_untracked();
        spawn_local(async move {
            match post_json(
                "/reasoning",
                &SetReasoningEnabledRequest {
                    id: Some(request_id),
                    enabled,
                    session_dir,
                },
            )
            .await
            {
                Ok(output) => {
                    if !handle_control_response(
                        output,
                        self.set_transport_status,
                        "Reasoning update failed",
                    ) {
                        self.set_reasoning_enabled.set(previous_enabled);
                        self.set_effort.set(previous_effort);
                    }
                }
                Err(error) => {
                    self.set_reasoning_enabled.set(previous_enabled);
                    self.set_effort.set(previous_effort);
                    self.set_control_error("Reasoning update failed", error);
                }
            }
        });
    }

    pub(crate) fn set_reasoning_effort(self, new_effort: ReasoningEffort) {
        if !self.reasoning_enabled.get() {
            return;
        }
        let previous_effort = self.effort.get();
        if previous_effort == new_effort {
            return;
        }
        let effort_value = new_effort.effort();
        self.set_effort.set(new_effort);
        let request_id = take_request_id(self.next_request_id, self.set_next_request_id, "effort");
        let session_dir = self.active_session_dir.get_untracked();
        spawn_local(async move {
            match post_json(
                "/effort",
                &SetReasoningEffortRequest {
                    id: Some(request_id),
                    effort: effort_value,
                    session_dir,
                },
            )
            .await
            {
                Ok(output) => {
                    if !handle_control_response(
                        output,
                        self.set_transport_status,
                        "Effort update failed",
                    ) {
                        self.set_effort.set(previous_effort);
                    }
                }
                Err(error) => {
                    self.set_effort.set(previous_effort);
                    self.set_control_error("Effort update failed", error);
                }
            }
        });
    }

    pub(crate) fn send_prompt(self, text: String, forced_mode: Option<PermissionMode>) {
        let text = text.trim().to_owned();
        if text.is_empty() || self.is_sending.get() {
            return;
        }

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
        let mode_request_id = forced_mode
            .map(|_| take_request_id(self.next_request_id, self.set_next_request_id, "mode"));
        let request_id = take_request_id(self.next_request_id, self.set_next_request_id, "send");
        let turn_id = request_id.clone();
        let session_dir = self.active_session_dir.get_untracked();
        self.set_active_turn_id.set(Some(turn_id.clone()));

        spawn_local(async move {
            if let Some(new_mode) = forced_mode {
                match post_json(
                    "/mode",
                    &SetPermissionModeRequest {
                        id: mode_request_id,
                        mode: new_mode,
                        session_dir: session_dir.clone(),
                    },
                )
                .await
                {
                    Ok(output) => {
                        if !self.is_active_turn(&turn_id) {
                            return;
                        }
                        let ok = command_succeeded(&output);
                        handle_command_response(
                            output,
                            self.set_messages,
                            self.next_message_id,
                            self.set_next_message_id,
                            self.set_transport_status,
                        );
                        if !ok {
                            self.finish_turn();
                            return;
                        }
                    }
                    Err(error) => {
                        if !self.is_active_turn(&turn_id) {
                            return;
                        }
                        self.finish_turn();
                        self.push_error("Mode update failed", error);
                        return;
                    }
                }
            }

            match post_json(
                "/send-async",
                &SendRequest {
                    id: Some(request_id),
                    text,
                    session_dir,
                },
            )
            .await
            {
                Ok(output) => {
                    if !self.is_active_turn(&turn_id) {
                        return;
                    }
                    if command_succeeded(&output) {
                        self.set_transport_status.set(TransportStatus::Connected);
                    } else {
                        self.finish_turn();
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
                    if !self.is_active_turn(&turn_id) {
                        return;
                    }
                    self.finish_turn();
                    self.push_error("Send failed", error);
                }
            }
        });
    }

    fn finish_turn(self) {
        self.set_is_sending.set(false);
        self.set_active_turn_id.set(None);
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

    fn is_active_turn(self, turn_id: &str) -> bool {
        self.active_turn_id.get().as_deref() == Some(turn_id)
    }

    fn set_control_error(self, prefix: &str, error: String) {
        self.set_transport_status
            .set(TransportStatus::Error(format!("{prefix}: {error}")));
    }
}

fn handle_control_response(
    output: StdioOutput,
    set_transport_status: WriteSignal<TransportStatus>,
    prefix: &str,
) -> bool {
    match output {
        StdioOutput::Response { ok: true, .. } => {
            set_transport_status.set(TransportStatus::Connected);
            true
        }
        StdioOutput::Response { error, .. } => {
            let message = error.unwrap_or_else(|| "request failed".to_owned());
            set_transport_status.set(TransportStatus::Error(format!("{prefix}: {message}")));
            false
        }
        StdioOutput::Event { .. } => {
            set_transport_status.set(TransportStatus::Error(format!(
                "{prefix}: unexpected event response"
            )));
            false
        }
    }
}

pub(crate) fn handle_command_response(
    output: StdioOutput,
    set_messages: WriteSignal<Vec<Message>>,
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
pub(crate) fn cancel_active_turn(
    active_turn_id: ReadSignal<Option<String>>,
    next_request_id: ReadSignal<u64>,
    set_next_request_id: WriteSignal<u64>,
    set_is_sending: WriteSignal<bool>,
    set_active_turn_id: WriteSignal<Option<String>>,
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
) {
    let Some(target_id) = active_turn_id.get() else {
        return;
    };
    let request_id = take_request_id(next_request_id, set_next_request_id, "cancel");
    spawn_local(async move {
        match post_json(
            "/cancel",
            &CancelRequest {
                id: Some(request_id),
                target_id,
            },
        )
        .await
        {
            Ok(output) => {
                set_is_sending.set(false);
                set_active_turn_id.set(None);
                handle_command_response(
                    output,
                    set_messages,
                    next_message_id,
                    set_next_message_id,
                    set_transport_status,
                );
            }
            Err(error) => {
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
        actions.send_prompt(text, None);
    }
}

pub(crate) fn send_planning_request(actions: AppActions, text: String) {
    let prompt = planning_prompt(&text);
    actions.send_prompt(prompt, Some(PermissionMode::Plan));
}

pub(crate) fn revise_plan_prompt(feedback: &str) -> String {
    format!(
        "Revise the latest plan using this feedback:\n\n{feedback}\n\nStay in read-only planning mode and return the updated staged plan."
    )
}

pub(crate) fn execute_plan_prompt() -> String {
    "Execute the latest approved plan from this transcript. If the plan is stale, unsafe, or underspecified, stop and explain what needs to change before execution.".to_owned()
}

pub(crate) fn take_request_id(
    next_request_id: ReadSignal<u64>,
    set_next_request_id: WriteSignal<u64>,
    prefix: &str,
) -> String {
    let id = next_request_id.get();
    set_next_request_id.set(id + 1);
    format!("{prefix}-{id}")
}

fn command_succeeded(output: &StdioOutput) -> bool {
    matches!(output, StdioOutput::Response { ok: true, .. })
}

fn planning_prompt(topic: &str) -> String {
    format!(
        "Plan mode topic:\n\n{topic}\n\nRun a planning interview before implementation. Stay read-only. First inspect only if useful, then ask the user 1-3 concise typed questions with 2-4 concrete options via request_user_input/AskUserQuestion whenever product, scope, UX, architecture, risk, or priority choices are missing. Put the recommended option first. Do not include an Other option because the client adds free-form Other automatically. Do not write files. After the user answers, return a staged implementation plan with assumptions, target files, verification, and unresolved risks."
    )
}
