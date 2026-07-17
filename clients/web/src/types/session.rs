use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct SessionSummary {
    pub(crate) session_dir: String,
    pub(crate) session_id: String,
    pub(crate) workspace_path: String,
    pub(crate) message_count: usize,
    pub(crate) updated_at_ms: Option<u64>,
    pub(crate) preview: Option<String>,
    pub(crate) activity: Option<SessionActivityInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct SessionActivityInfo {
    pub(crate) status: String,
    pub(crate) running_turns: usize,
    pub(crate) running_turn_ids: Vec<String>,
    pub(crate) pending_approvals: usize,
    pub(crate) pending_user_inputs: usize,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct TranscriptMessage {
    pub(crate) role: String,
    pub(crate) text: String,
    pub(crate) tool: Option<TranscriptTool>,
    pub(crate) subagent: Option<TranscriptSubagent>,
    /// Хвост незавершённого хода: текст ещё стримится, клиент должен сделать
    /// это сообщение целью для последующих SSE-дельт.
    pub(crate) streaming: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct TranscriptTool {
    pub(crate) call_id: String,
    pub(crate) name: String,
    pub(crate) args: Value,
    pub(crate) status: String,
    pub(crate) result: Option<String>,
    /// `ToolResult.metadata` как есть — из неё клиент строит спец-рендеры
    /// (карточка субагента по результату `task`).
    pub(crate) metadata: Value,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct TranscriptSubagent {
    pub(crate) child_thread_id: String,
    pub(crate) role: String,
    pub(crate) description: Option<String>,
    pub(crate) status: String,
    pub(crate) iterations: Option<u32>,
    pub(crate) tools: Vec<TranscriptTool>,
}
