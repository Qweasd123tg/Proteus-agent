use super::*;

impl AppActions {
    pub(crate) async fn edit_queued_prompt(
        self,
        message_id: String,
        text: String,
    ) -> Result<(), String> {
        let session_dir = self
            .active_session_dir
            .get_untracked()
            .ok_or_else(|| "Сессия ещё не выбрана.".to_owned())?;
        let generation = self.transcript_generation.get_untracked();
        let output = post_json(
            "/queue/edit",
            &EditQueuedMessageRequest {
                id: Some(take_request_id(
                    self.next_request_id,
                    self.set_next_request_id,
                    "queue-edit",
                )),
                session_dir: session_dir.clone(),
                message_id: message_id.clone(),
                text: text.clone(),
            },
        )
        .await?;
        if !self.is_current_session(&session_dir, generation) {
            return Ok(());
        }
        queue_command_result(output)?;
        // The ordered event stream supplies the text. A late HTTP reply must
        // not overwrite a subsequent edit or recreate a delivered message.
        Ok(())
    }

    pub(crate) async fn delete_queued_prompt(self, message_id: String) -> Result<(), String> {
        let session_dir = self
            .active_session_dir
            .get_untracked()
            .ok_or_else(|| "Сессия ещё не выбрана.".to_owned())?;
        let generation = self.transcript_generation.get_untracked();
        let output = post_json(
            "/queue/delete",
            &DeleteQueuedMessageRequest {
                id: Some(take_request_id(
                    self.next_request_id,
                    self.set_next_request_id,
                    "queue-delete",
                )),
                session_dir: session_dir.clone(),
                message_id: message_id.clone(),
            },
        )
        .await?;
        if !self.is_current_session(&session_dir, generation) {
            return Ok(());
        }
        queue_command_result(output)?;
        self.set_queued_prompts
            .update(|items| items.retain(|item| item.message_id != message_id));
        Ok(())
    }
    /// Отправляет уточнение во время активного root turn-а. Сервер сразу
    /// принимает его в session-owned очередь; локальный клиент не решает,
    /// станет сообщение steering или follow-up.
    pub(crate) fn queue_prompt(self, text: String) {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        let request_id = take_request_id(self.next_request_id, self.set_next_request_id, "steer");
        let Some(session_dir) = self.active_session_dir.get_untracked() else {
            return;
        };
        let generation = self.transcript_generation.get_untracked();
        let submitted_text = text.clone();
        spawn_local(async move {
            match post_json(
                "/send-async",
                &SendRequest {
                    id: Some(request_id.clone()),
                    text,
                    session_dir: session_dir.clone(),
                },
            )
            .await
            {
                Ok(StdioOutput::Response {
                    ok: true, output, ..
                }) => {
                    if !self.is_current_session(&session_dir, generation) {
                        return;
                    }
                    self.set_transport_status.set(TransportStatus::Connected);
                    let queued = output
                        .as_ref()
                        .and_then(|value| value.get("queued"))
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    // Queue contents arrive through SSE (or /pending on reconnect).
                    // A delayed acceptance reply must not revive a delivered row.
                    if !queued {
                        // Race: предыдущий turn успел завершиться до запроса,
                        // поэтому runtime зарезервировал полноценный новый.
                        self.set_is_sending.set(true);
                        self.set_active_run_id.set(Some(request_id));
                        push_user_message_once(
                            self.set_messages,
                            self.next_message_id,
                            self.set_next_message_id,
                            submitted_text,
                        );
                    }
                }
                Ok(output) => {
                    if self.is_current_session(&session_dir, generation) {
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
                    if self.is_current_session(&session_dir, generation) {
                        self.push_error("Queue send failed", error);
                    }
                }
            }
        });
    }
}

fn queue_command_result(output: StdioOutput) -> Result<(), String> {
    match output {
        StdioOutput::Response { ok: true, .. } => Ok(()),
        StdioOutput::Response { error, .. } => Err(match error {
            Some(error) if error.contains("no longer pending") => {
                "Сообщение уже передано агенту или удалено.".to_owned()
            }
            Some(error) => format!("Не удалось изменить очередь: {error}"),
            None => "Не удалось изменить очередь.".to_owned(),
        }),
        _ => Err("Сервер не подтвердил изменение очереди.".to_owned()),
    }
}
