use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct SessionSummary {
    pub(crate) session_dir: String,
    pub(crate) session_id: Option<String>,
    pub(crate) workspace_path: Option<String>,
    pub(crate) message_count: usize,
    pub(crate) updated_at_ms: Option<u64>,
    pub(crate) preview: Option<String>,
    pub(crate) resumable: bool,
    #[serde(default)]
    pub(crate) activity: Option<SessionActivityInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct SessionActivityInfo {
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) running_turns: usize,
    #[serde(default)]
    pub(crate) running_turn_ids: Vec<String>,
    #[serde(default)]
    pub(crate) pending_approvals: usize,
    #[serde(default)]
    pub(crate) pending_user_inputs: usize,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct TranscriptMessage {
    pub(crate) role: String,
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) tool: Option<TranscriptTool>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct TranscriptTool {
    pub(crate) call_id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) args: Value,
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) result: Option<String>,
}
