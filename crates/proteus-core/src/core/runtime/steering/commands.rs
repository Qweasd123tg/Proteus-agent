use super::*;
use crate::core::AgentRuntime;

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;

impl SessionSteering {
    /// The same lock arbitrates editing/removal and delivery. Once popped, a
    /// message cannot be changed, recreated or mistaken for a pending one.
    fn update_pending(
        &self,
        message_id: MessageId,
        text: Option<String>,
    ) -> Result<SteeringQueueReceipt> {
        if let Some(text) = &text {
            validate_message(text)?;
        }
        let mut state = self.state.lock().expect("steering state lock");
        let index = state
            .queued
            .iter()
            .position(|item| item.message.id == message_id)
            .ok_or_else(|| anyhow::anyhow!("queued message is no longer pending"))?;
        let active_turn_id = state
            .active_turn_id
            .ok_or_else(|| anyhow::anyhow!("queued message has no active turn"))?;
        let old_len = state.queued[index].text.len();
        let result_text = if let Some(text) = text {
            let next_bytes = state.queued_bytes - old_len + text.len();
            ensure!(
                next_bytes <= MAX_QUEUED_BYTES,
                "root steering queue byte budget exceeded (max {MAX_QUEUED_BYTES} bytes)"
            );
            let queued = &mut state.queued[index];
            queued.message =
                CanonicalMessage::text(MessageRole::User, text.clone()).with_id(message_id);
            queued.text = text.clone();
            state.queued_bytes = next_bytes;
            text
        } else {
            let queued = state.queued.remove(index).expect("pending index");
            state.queued_bytes -= old_len;
            self.queued_count
                .store(state.queued.len(), Ordering::Release);
            queued.text
        };
        self.publish_queue_snapshot(&state);
        Ok(SteeringQueueReceipt {
            message_id,
            text: result_text,
            active_turn_id,
            queued_count: state.queued.len(),
        })
    }
}

impl AgentRuntime {
    pub(crate) async fn edit_queued_user_message(
        &self,
        message_id: MessageId,
        text: String,
    ) -> Result<()> {
        let receipt = self
            .session
            .steering
            .update_pending(message_id, Some(text))?;
        self.services
            .events
            .emit(
                EventContext::new(
                    self.session.session_id,
                    self.session.thread_id,
                    Some(receipt.active_turn_id),
                ),
                Event::SteeringEdited {
                    message_id,
                    text: receipt.text,
                    queued_count: receipt.queued_count,
                },
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn delete_queued_user_message(&self, message_id: MessageId) -> Result<()> {
        let receipt = self.session.steering.update_pending(message_id, None)?;
        self.services
            .events
            .emit(
                EventContext::new(
                    self.session.session_id,
                    self.session.thread_id,
                    Some(receipt.active_turn_id),
                ),
                Event::SteeringRemoved {
                    message_id,
                    queued_count: receipt.queued_count,
                },
            )
            .await?;
        Ok(())
    }
}
