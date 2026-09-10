use crate::model_standard::MessagePhase;
use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppTranscriptMessage {
    pub message_id: Option<crate::domain::MessageId>,
    pub phase: Option<MessagePhase>,
    pub role: String,
    pub text: String,
    pub tool: Option<AppTranscriptTool>,
    pub subagent: Option<AppTranscriptSubagent>,
    /// Текст ещё стримится: сообщение — живой прогресс незавершённого хода
    /// (см. turn_progress), клиент продолжает дописывать в него дельты.
    pub streaming: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppTranscriptTool {
    pub call_id: String,
    pub name: String,
    pub args: Value,
    pub status: String,
    pub result: Option<String>,
    /// Metadata результата как есть (`ToolResult.metadata`): core не знает
    /// конкретных tools, а клиенты по ней строят спец-рендеры (например,
    /// карточку субагента из результата `task`).
    pub metadata: Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppTranscriptSubagent {
    pub child_thread_id: String,
    pub role: String,
    pub description: Option<String>,
    pub status: String,
    pub iterations: Option<u32>,
    pub tools: Vec<AppTranscriptTool>,
}
