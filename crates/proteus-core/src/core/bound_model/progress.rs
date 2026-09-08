use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};

use crate::{
    domain::MessageId,
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, ContentPart, MessageRole, ModelFailure,
    },
};

/// Completed assistant text accepted before a terminal model failure.
#[derive(Clone)]
pub(super) struct CompletedMessageProgress {
    request_ids: HashSet<MessageId>,
    messages: Vec<CanonicalMessage>,
    positions: HashMap<MessageId, usize>,
}

impl CompletedMessageProgress {
    pub(super) fn new(request: &CanonicalModelRequest) -> Self {
        Self {
            request_ids: request.messages.iter().map(|message| message.id).collect(),
            messages: Vec::new(),
            positions: HashMap::new(),
        }
    }

    pub(super) fn accept(&mut self, message: CanonicalMessage) -> Result<()> {
        if message.role != MessageRole::Assistant {
            bail!("completed model message must have the assistant role");
        }
        if self.request_ids.contains(&message.id) {
            bail!(
                "completed model message id {} conflicts with the request history",
                message.id
            );
        }
        if message.parts.iter().any(|part| {
            matches!(
                part.payload,
                ContentPart::ToolCall { .. } | ContentPart::ToolResult { .. }
            )
        }) {
            bail!(
                "completed model message {} cannot contain a tool call or result",
                message.id
            );
        }
        if let Some(index) = self.positions.get(&message.id).copied() {
            if !same_message_ignoring_part_ids(&self.messages[index], &message) {
                bail!("completed model message id {} was reused", message.id);
            }
            return Ok(());
        }
        self.positions.insert(message.id, self.messages.len());
        self.messages.push(message);
        Ok(())
    }

    pub(super) fn attach_to_failure(&mut self, mut failure: ModelFailure) -> ModelFailure {
        let mut candidate = self.clone();
        for message in std::mem::take(&mut failure.completed_messages) {
            if let Err(error) = candidate.accept(message) {
                return ModelFailure::other(format!("model protocol error: {error}"))
                    .with_completed_messages(self.messages.clone());
            }
        }
        *self = candidate;
        failure.completed_messages = self.messages.clone();
        failure
    }
}

fn same_message_ignoring_part_ids(left: &CanonicalMessage, right: &CanonicalMessage) -> bool {
    left.role == right.role
        && left.phase == right.phase
        && left.name == right.name
        && left.tool_call_id == right.tool_call_id
        && left.metadata == right.metadata
        && left.parts.len() == right.parts.len()
        && left.parts.iter().zip(&right.parts).all(|(left, right)| {
            left.provenance == right.provenance
                && left.scope == right.scope
                && left.payload == right.payload
        })
}
