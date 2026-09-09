use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};

use crate::{
    domain::{CallId, MessageId, PartId, ToolSpec},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, ContentPart, MessageRole, ModelFailure, PartScope,
        validate_model_tool_call_surface,
    },
};

/// Completed assistant items accepted before a terminal model failure.
#[derive(Clone)]
pub(super) struct CompletedMessageProgress {
    request_ids: HashSet<MessageId>,
    part_ids: HashSet<PartId>,
    messages: Vec<CanonicalMessage>,
    positions: HashMap<MessageId, usize>,
    call_ids: HashSet<CallId>,
    tools: Vec<ToolSpec>,
}

impl CompletedMessageProgress {
    pub(super) fn new(request: &CanonicalModelRequest) -> Self {
        Self {
            request_ids: request.messages.iter().map(|message| message.id).collect(),
            part_ids: request
                .messages
                .iter()
                .flat_map(|message| message.parts.iter().map(|part| part.part_id))
                .collect(),
            messages: Vec::new(),
            positions: HashMap::new(),
            call_ids: request
                .messages
                .iter()
                .flat_map(|message| &message.parts)
                .filter_map(|part| match &part.payload {
                    ContentPart::ToolCall { call } => Some(call.id.clone()),
                    ContentPart::ToolResult { result } => Some(result.call_id.clone()),
                    _ => None,
                })
                .collect(),
            tools: request.tools.clone(),
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
        if message
            .parts
            .iter()
            .any(|part| matches!(part.payload, ContentPart::ToolResult { .. }))
        {
            bail!(
                "completed model message {} cannot contain a tool result",
                message.id
            );
        }
        if let Some(index) = self.positions.get(&message.id).copied() {
            if !same_message_ignoring_part_ids(&self.messages[index], &message) {
                bail!("completed model message id {} was reused", message.id);
            }
            return Ok(());
        }
        // A completed item must be safe to retain as conversation history.
        // Check before accepting it so malformed progress cannot invalidate a
        // later workflow checkpoint containing previously accepted messages.
        let mut message_part_ids = HashSet::new();
        let mut message_call_ids = HashSet::new();
        for part in &message.parts {
            if part.scope != PartScope::Conversation {
                bail!(
                    "completed model message {} contains non-conversation part {}",
                    message.id,
                    part.part_id
                );
            }
            if self.part_ids.contains(&part.part_id) || !message_part_ids.insert(part.part_id) {
                bail!(
                    "completed model message {} contains reused part id {}",
                    message.id,
                    part.part_id
                );
            }
            if let ContentPart::ToolCall { call } = &part.payload {
                validate_model_tool_call_surface(&self.tools, call).map_err(anyhow::Error::msg)?;
                if self.call_ids.contains(&call.id) || !message_call_ids.insert(call.id.clone()) {
                    bail!(
                        "completed model message {} reused tool call id '{}'",
                        message.id,
                        call.id
                    );
                }
            }
        }
        self.part_ids.extend(message_part_ids);
        self.call_ids.extend(message_call_ids);
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
