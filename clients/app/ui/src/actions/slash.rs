use super::*;
use proteus_contracts::app_protocol::{StdioRequest, commands::CommandOutput};

impl AppActions {
    pub(crate) fn execute_slash(
        self,
        text: String,
        draft: ReadSignal<String>,
        set_draft: WriteSignal<String>,
    ) {
        let Some(session) = self.active_session_dir.get_untracked() else {
            return;
        };
        if self.controls.is_pending() {
            self.push_error(
                "Command failed",
                "Дождитесь завершения предыдущей команды или смены настроек".into(),
            );
            return;
        }
        if self.attachments_loading.get_untracked()
            || self.attachments.with_untracked(|images| !images.is_empty())
        {
            self.push_error(
                "Command failed",
                "Отправьте вложения отдельным сообщением".into(),
            );
            return;
        }
        let generation = self.transcript_generation.get_untracked();
        let revision = self.controls.begin();
        let original = draft.get_untracked();
        let request = StdioRequest::ExecuteCommand {
            id: Some(take_request_id(
                self.next_request_id,
                self.set_next_request_id,
                "command",
            )),
            text,
        };
        self.controls.enqueue(async move {
            if !self.is_current_session(&session, generation) {
                return;
            }
            let response =
                post_json(&crate::api::session_path("/request", &session), &request).await;
            if !self.is_current_session(&session, generation) {
                return;
            }
            let output = match response {
                Ok(StdioOutput::Response {
                    ok: true,
                    output: Some(value),
                    ..
                }) => serde_json::from_value::<CommandOutput>(value).map_err(|e| e.to_string()),
                Ok(StdioOutput::Response { error, .. }) => {
                    Err(error.unwrap_or_else(|| "command rejected".into()))
                }
                Ok(_) => Err("server did not confirm the command".into()),
                Err(error) => Err(error),
            };
            match output {
                Ok(CommandOutput::Display { text }) => {
                    if draft.get_untracked() == original {
                        set_draft.set(String::new());
                    }
                    push_message(
                        self.set_messages,
                        self.next_message_id,
                        self.set_next_message_id,
                        MessageRole::System,
                        text,
                    );
                    self.set_transport_status.set(TransportStatus::Connected);
                }
                Ok(CommandOutput::Prompt { text }) => {
                    if draft.get_untracked() == original {
                        set_draft.set(String::new());
                    }
                    if self.is_sending.get_untracked() {
                        self.queue_prompt(text);
                    } else {
                        send_prompt_for_mode(self, self.mode.get_untracked(), text);
                    }
                }
                Err(error) => {
                    self.push_error("Command failed", error);
                    return;
                }
            }
            // Serialize slash controls with the model/mode dropdowns. A late
            // response must not overwrite a newer selection or another session.
            let config = crate::api::get_json::<
                proteus_contracts::app_protocol::config::ConfigSummary,
            >(&crate::api::session_path("/config", &session))
            .await;
            if self.is_current_session(&session, generation) && self.controls.current(revision) {
                match config {
                    Ok(config) => {
                        crate::model_settings::ModelSettings {
                            model: self.set_model_name,
                            models: self.set_model_options,
                            enabled: self.set_reasoning_enabled,
                            effort: self.set_effort,
                            efforts: self.set_effort_options,
                            status: self.set_transport_status,
                        }
                        .apply(&config);
                        self.set_mode
                            .set(PermissionMode::from_value(&config.permission_mode));
                        self.remember_selection(&session, generation).await;
                    }
                    Err(error) => self.set_control_error("Command settings refresh failed", error),
                }
            }
        });
    }
}
