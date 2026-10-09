//! Root-owned read capability. No session lookup or cross-thread access.
use crate::{
    contracts::{ConversationReader, ConversationSnapshot},
    model_standard::CanonicalMessage,
};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(super) struct TurnConversation {
    pub history: Arc<Mutex<Vec<CanonicalMessage>>>,
    pub context: Arc<Mutex<crate::core::model_context::ModelContextState>>,
}

#[async_trait]
impl ConversationReader for TurnConversation {
    async fn read(&self) -> Result<ConversationSnapshot> {
        // Use checkpoint lock order: history before context.
        let messages = self.history.lock().await;
        let context = self.context.lock().await;
        Ok(ConversationSnapshot {
            messages: messages.clone(),
            model_context: context.snapshot(),
        })
    }
}
