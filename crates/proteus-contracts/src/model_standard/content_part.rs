use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::{
    CallId, Citation, ContextChunk, HostedToolActivity, MessageId, PartId, Patch, ToolCall,
    ToolResult,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum MessageRole {
    System,
    Developer,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessagePhase {
    Commentary,
    FinalAnswer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct CanonicalMessage {
    pub id: MessageId,
    pub role: MessageRole,
    /// Provider-defined lifecycle phase for assistant output. `None` means
    /// that the provider did not classify the message; consumers keep the
    /// ordinary final-message fallback for such models.
    pub phase: Option<MessagePhase>,
    pub parts: Vec<CanonicalPart>,
    pub name: Option<String>,
    pub tool_call_id: Option<CallId>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PartProvenance {
    User,
    Model,
    Tool,
    ContextBuilder,
    Compactor,
    Runtime,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PartScope {
    Conversation,
    Request,
    Trace,
}

/// Stable canonical part record. Storage and projections use the explicit
/// provenance/scope fields instead of inferring semantics from message names
/// or unstructured metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CanonicalPart {
    pub part_id: PartId,
    pub provenance: PartProvenance,
    pub scope: PartScope,
    pub payload: ContentPart,
}

impl CanonicalPart {
    pub fn new(provenance: PartProvenance, scope: PartScope, payload: ContentPart) -> Self {
        Self {
            part_id: crate::domain::new_part_id(),
            provenance,
            scope,
            payload,
        }
    }

    pub fn with_id(mut self, part_id: PartId) -> Self {
        self.part_id = part_id;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum ContentPart {
    Text {
        text: String,
    },
    Context {
        chunk: ContextChunk,
    },
    FileRef {
        path: PathBuf,
        content: Option<String>,
    },
    ToolCall {
        call: ToolCall,
    },
    ToolResult {
        result: ToolResult,
    },
    Patch {
        patch: Patch,
    },
    ReasoningSummary {
        text: String,
    },
    Reasoning {
        text: String,
        signature: Option<String>,
    },
    HostedToolActivity {
        activity: HostedToolActivity,
    },
    Citation {
        citation: Citation,
    },
}

impl CanonicalMessage {
    pub fn text(role: MessageRole, text: impl Into<String>) -> Self {
        Self::new(role, vec![ContentPart::Text { text: text.into() }])
    }

    /// Сообщение с произвольными parts. Остальные поля можно выставить
    /// через `with_*` helpers.
    pub fn new(role: MessageRole, parts: Vec<ContentPart>) -> Self {
        let parts = parts
            .into_iter()
            .map(|payload| {
                let (provenance, scope) = default_part_semantics(&role, &payload);
                CanonicalPart::new(provenance, scope, payload)
            })
            .collect();
        Self::from_parts(role, parts)
    }

    /// Конструктор для уже размеченных canonical parts. Используется там,
    /// где provenance/scope задаёт конкретная lifecycle boundary.
    pub fn from_parts(role: MessageRole, parts: Vec<CanonicalPart>) -> Self {
        Self {
            id: crate::domain::new_message_id(),
            role,
            phase: None,
            parts,
            name: None,
            tool_call_id: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_id(mut self, id: MessageId) -> Self {
        self.id = id;
        self
    }

    pub fn with_phase(mut self, phase: MessagePhase) -> Self {
        self.phase = Some(phase);
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_tool_call_id(mut self, id: CallId) -> Self {
        self.tool_call_id = Some(id);
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

fn default_part_semantics(
    role: &MessageRole,
    payload: &ContentPart,
) -> (PartProvenance, PartScope) {
    match payload {
        ContentPart::Context { .. } => (PartProvenance::ContextBuilder, PartScope::Request),
        ContentPart::ToolResult { .. } | ContentPart::Patch { .. } => {
            (PartProvenance::Tool, PartScope::Conversation)
        }
        ContentPart::ToolCall { .. }
        | ContentPart::ReasoningSummary { .. }
        | ContentPart::Reasoning { .. }
        | ContentPart::HostedToolActivity { .. }
        | ContentPart::Citation { .. } => (PartProvenance::Model, PartScope::Conversation),
        ContentPart::Text { .. } | ContentPart::FileRef { .. } => match role {
            MessageRole::User => (PartProvenance::User, PartScope::Conversation),
            MessageRole::Assistant => (PartProvenance::Model, PartScope::Conversation),
            MessageRole::Tool => (PartProvenance::Tool, PartScope::Conversation),
            MessageRole::System | MessageRole::Developer => {
                (PartProvenance::Runtime, PartScope::Conversation)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ContextChunk;

    #[test]
    fn constructors_assign_explicit_part_semantics() {
        let user = CanonicalMessage::text(MessageRole::User, "hello");
        let context = CanonicalMessage::new(
            MessageRole::User,
            vec![ContentPart::Context {
                chunk: ContextChunk::new("repo", "context"),
            }],
        );

        assert_eq!(user.parts[0].provenance, PartProvenance::User);
        assert_eq!(user.parts[0].scope, PartScope::Conversation);
        assert_eq!(context.parts[0].provenance, PartProvenance::ContextBuilder);
        assert_eq!(context.parts[0].scope, PartScope::Request);
    }

    #[test]
    fn part_record_round_trip_preserves_stable_id_and_rejects_unknown_fields() {
        let message = CanonicalMessage::text(MessageRole::Assistant, "answer");
        let value = serde_json::to_value(&message).expect("serialize");
        let round_trip: CanonicalMessage =
            serde_json::from_value(value.clone()).expect("deserialize");

        assert_eq!(round_trip, message);
        assert!(value["parts"][0].get("part_id").is_some());
        assert_eq!(value["parts"][0]["provenance"], "model");
        assert_eq!(value["parts"][0]["scope"], "conversation");

        let mut invalid = value;
        invalid["parts"][0]["unknown"] = serde_json::json!(true);
        assert!(serde_json::from_value::<CanonicalMessage>(invalid).is_err());
    }
}
