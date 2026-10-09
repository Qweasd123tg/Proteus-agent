//! Read-only conversation facts, available only in conversation-bound operations.
use anyhow::{Result, ensure};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::ModelContextObservation;
use crate::{
    domain::{MessageId, ToolCall},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};

pub const TOOL_HOST_READ_CONVERSATION_METHOD: &str = "host.conversation.read";
pub const TOOL_HOST_CONVERSATION_SNAPSHOT_METHOD: &str = "host.conversation.snapshot";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionConversationSnapshot {
    pub session_id: crate::domain::SessionId,
    pub conversation: ConversationSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSnapshot {
    pub messages: Vec<CanonicalMessage>,
    pub model_context: Vec<ModelContextObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolConversationSnapshot {
    pub conversation: ConversationSnapshot,
    pub message_id: MessageId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadConversationRequest {}

impl ConversationSnapshot {
    /// Resolve identity, not edited tool arguments: a workflow may explicitly
    /// select a different execution call for this canonical history call.
    pub fn for_tool(self, call: &ToolCall) -> Result<ToolConversationSnapshot> {
        let mut owners = self.messages.iter().filter(|message| {
            message.role == MessageRole::Assistant && message.parts.iter().any(|part| {
                matches!(&part.payload, ContentPart::ToolCall { call: original } if original.id == call.id)
            })
        });
        let message_id = owners
            .next()
            .ok_or_else(|| anyhow::anyhow!("tool call has no checkpointed conversation message"))?
            .id;
        ensure!(
            owners.next().is_none(),
            "tool call has ambiguous conversation identity"
        );
        Ok(ToolConversationSnapshot {
            conversation: self,
            message_id,
        })
    }
}

#[async_trait]
pub trait ConversationReader: Send + Sync {
    async fn read(&self) -> Result<ConversationSnapshot>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::new_call_id;

    #[test]
    fn read_only_snapshot_resolves_canonical_owner_not_rewritten_args_and_rejects_ambiguity() {
        let call = ToolCall::new(
            new_call_id(),
            "compress",
            serde_json::json!({"original": true}),
        );
        let message = CanonicalMessage::new(
            MessageRole::Assistant,
            vec![ContentPart::ToolCall { call: call.clone() }],
        );
        let snapshot = ConversationSnapshot {
            messages: vec![message.clone()],
            model_context: vec![],
        };
        let mut execution_call = call.clone();
        execution_call.args = serde_json::json!({"edited": true});
        assert_eq!(
            snapshot
                .clone()
                .for_tool(&execution_call)
                .unwrap()
                .message_id,
            message.id
        );
        let mut ambiguous = snapshot.clone();
        ambiguous.messages.push(message);
        assert!(
            ambiguous
                .for_tool(&call)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        assert!(
            snapshot
                .for_tool(&ToolCall::new(
                    new_call_id(),
                    "compress",
                    serde_json::json!({})
                ))
                .is_err()
        );
        assert!(
            serde_json::from_value::<ReadConversationRequest>(
                serde_json::json!({"session_id":"other"})
            )
            .is_err()
        );
    }
}
