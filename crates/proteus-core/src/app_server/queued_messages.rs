use super::AppServerHandle;
use crate::domain::MessageId;
use anyhow::Result;

impl AppServerHandle {
    pub async fn edit_queued_user_message(
        &self,
        message_id: MessageId,
        text: String,
    ) -> Result<()> {
        self.runtime
            .edit_queued_user_message(message_id, text)
            .await
    }

    pub async fn delete_queued_user_message(&self, message_id: MessageId) -> Result<()> {
        self.runtime.delete_queued_user_message(message_id).await
    }
}
