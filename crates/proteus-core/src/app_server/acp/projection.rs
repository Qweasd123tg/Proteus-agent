//! Append-only ACP projection of one canonical root turn.
use crate::domain::{Event, EventEnvelope, MessageId, SessionId as RuntimeSessionId, ThreadId};
use agent_client_protocol::{Result, schema::v1::*};
use std::collections::{HashMap, HashSet};

pub(super) struct Projection {
    session: RuntimeSessionId,
    root: Option<ThreadId>,
    texts: HashMap<MessageId, String>,
    last_message: Option<MessageId>,
    tools: HashSet<String>,
}

impl Projection {
    pub(super) fn new(session: RuntimeSessionId) -> Self {
        Self {
            session,
            root: None,
            texts: HashMap::new(),
            last_message: None,
            tools: HashSet::new(),
        }
    }

    pub(super) fn event(&mut self, envelope: EventEnvelope) -> Result<Vec<SessionUpdate>> {
        if envelope.session_id != self.session {
            return Ok(vec![]);
        }
        if matches!(envelope.event, Event::TurnStarted { .. }) && self.root.is_none() {
            self.root = Some(envelope.thread_id);
        }
        if self.root != Some(envelope.thread_id) {
            return Ok(vec![]);
        }
        let update = match envelope.event {
            Event::AssistantTextDelta {
                message_id,
                offset,
                text,
                ..
            } => {
                let current = self.texts.entry(message_id).or_default();
                if offset > current.len() || !current.is_char_boundary(offset) {
                    return Err(super::internal("assistant stream has a gap"));
                }
                let overlap = current.len() - offset;
                let shared = overlap.min(text.len());
                if !current.is_char_boundary(offset + shared)
                    || !text.is_char_boundary(shared)
                    || current[offset..offset + shared] != text[..shared]
                {
                    return Err(super::internal(
                        "assistant stream changed already published text",
                    ));
                }
                let suffix = text[shared..].to_owned();
                current.push_str(&suffix);
                self.message_chunk(message_id, suffix)
            }
            Event::AssistantMessageCompleted {
                message_id, text, ..
            } => {
                let current = self.texts.entry(message_id).or_default();
                let Some(suffix) = text.strip_prefix(current.as_str()) else {
                    return Err(super::internal(
                        "completed assistant message changed published text",
                    ));
                };
                let suffix = suffix.to_owned();
                *current = text;
                self.message_chunk(message_id, suffix)
            }
            Event::AssistantReasoningDelta { text } => Some(SessionUpdate::AgentThoughtChunk(
                ContentChunk::new(ContentBlock::Text(TextContent::new(text))),
            )),
            Event::ToolCallRequested { call } => {
                self.tools.insert(call.id.to_string());
                Some(SessionUpdate::ToolCall(
                    ToolCall::new(call.id.to_string(), call.name)
                        .status(ToolCallStatus::Pending)
                        .raw_input(call.args),
                ))
            }
            Event::ApprovalResolved { call_id, approved } => {
                Some(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    call_id.to_string(),
                    ToolCallUpdateFields::new().status(if approved {
                        ToolCallStatus::InProgress
                    } else {
                        ToolCallStatus::Failed
                    }),
                )))
            }
            Event::ToolFinished { result } => {
                self.tools.remove(&result.call_id.to_string());
                let text = result
                    .error
                    .clone()
                    .unwrap_or_else(|| result.output.clone());
                Some(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    result.call_id.to_string(),
                    ToolCallUpdateFields::new()
                        .status(if result.ok {
                            ToolCallStatus::Completed
                        } else {
                            ToolCallStatus::Failed
                        })
                        .content(vec![ToolCallContent::Content(Content::new(
                            ContentBlock::Text(TextContent::new(text)),
                        ))])
                        .raw_output(serde_json::to_value(result).map_err(super::internal)?),
                )))
            }
            _ => None,
        };
        Ok(update.into_iter().collect())
    }

    pub(super) fn fallback_output(&self, text: String) -> Option<SessionUpdate> {
        if self.texts.values().all(String::is_empty) {
            message(text)
        } else {
            None
        }
    }

    fn message_chunk(&mut self, id: MessageId, text: String) -> Option<SessionUpdate> {
        if text.is_empty() {
            return None;
        }
        let separated = self.last_message.is_some_and(|previous| previous != id);
        self.last_message = Some(id);
        message(if separated {
            format!("\n\n{text}")
        } else {
            text
        })
    }

    pub(super) fn settle_tools(&mut self) -> Vec<SessionUpdate> {
        self.tools
            .drain()
            .map(|id| {
                SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    id,
                    ToolCallUpdateFields::new()
                        .status(ToolCallStatus::Failed)
                        .content(vec![ToolCallContent::Content(Content::new(
                            ContentBlock::Text(TextContent::new(
                                "Turn ended before this tool completed",
                            )),
                        ))]),
                ))
            })
            .collect()
    }
}

pub(super) fn message(text: String) -> Option<SessionUpdate> {
    (!text.is_empty()).then(|| {
        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            text,
        ))))
    })
}
