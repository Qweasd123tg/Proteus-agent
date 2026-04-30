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
}
