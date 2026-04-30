use serde::{Deserialize, Serialize};

use crate::domain::ids::CallId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: CallId,
    pub name: String,
    pub args: serde_json::Value,
}

impl ToolCall {
    pub fn new(id: impl Into<CallId>, name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            args,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub call_id: CallId,
    pub ok: bool,
    pub output: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ToolContent>,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
}

impl ToolResult {
    /// Успешный результат с текстовым выводом.
    pub fn ok(call_id: CallId, output: impl Into<String>) -> Self {
        Self {
            call_id,
            ok: true,
            output: output.into(),
            content: Vec::new(),
            error: None,
            metadata: serde_json::Value::Null,
        }
    }

    /// Результат-ошибка.
    pub fn error(call_id: CallId, error: impl Into<String>) -> Self {
        Self {
            call_id,
            ok: false,
            output: String::new(),
            content: Vec::new(),
            error: Some(error.into()),
            metadata: serde_json::Value::Null,
        }
    }

    /// Полный конструктор со всеми полями.
    pub fn new(
        call_id: CallId,
        ok: bool,
        output: String,
        content: Vec<ToolContent>,
        error: Option<String>,
        metadata: serde_json::Value,
    ) -> Self {
        Self {
            call_id,
            ok,
            output,
            content,
            error,
            metadata,
        }
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_content(mut self, content: Vec<ToolContent>) -> Self {
        self.content = content;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolContent {
    Text { text: String },
    Json { value: serde_json::Value },
    Image { mime_type: String, data: String },
    Binary { mime_type: String, data: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub safety: ToolSafety,
    pub timeout_ms: Option<u64>,
    pub metadata: serde_json::Value,
}

impl ToolSpec {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
        safety: ToolSafety,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            safety,
            timeout_ms: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolSafety {
    ReadOnly,
    WritesFiles,
    RunsCommands,
    Network,
    Dangerous,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PermissionMode {
    Plan,
    #[default]
    Normal,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolicyDecision {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}
