use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::{CallId, ContextChunk, MessageId, Patch, ToolCall, ToolResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum MessageRole {
    System,
    Developer,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct CanonicalMessage {
    pub id: MessageId,
    pub role: MessageRole,
    pub parts: Vec<ContentPart>,
    pub name: Option<String>,
    pub tool_call_id: Option<CallId>,
    pub metadata: serde_json::Value,
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
}

impl CanonicalMessage {
    pub fn text(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            id: crate::domain::new_message_id(),
            role,
            parts: vec![ContentPart::Text { text: text.into() }],
            name: None,
            tool_call_id: None,
            metadata: serde_json::Value::Null,
        }
    }

    /// Сообщение с произвольными parts. Остальные поля можно выставить
    /// через `with_*` helpers.
    pub fn new(role: MessageRole, parts: Vec<ContentPart>) -> Self {
        Self {
            id: crate::domain::new_message_id(),
            role,
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
