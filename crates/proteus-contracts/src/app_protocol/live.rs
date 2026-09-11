use super::AppTranscriptMessage;
use crate::domain::SessionId;
use serde::{Deserialize, Serialize};

/// Session subscription baseline. seq belongs to this live stream incarnation.
/// The server filters already included events before delivery to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSessionSnapshot {
    pub session_id: SessionId,
    pub stream_id: String,
    pub seq: u64,
    pub root_thread_id: Option<crate::domain::ThreadId>,
    pub transcript: Vec<AppTranscriptMessage>,
    pub execution: AppExecutionState,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppExecutionState {
    pub active: Option<AppRun>,
    pub last: Option<AppRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppRun {
    pub run_id: String,
    pub options: crate::domain::RunOptions,
    pub status: AppRunStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppRunStatus {
    Running,
    CancelRequested,
    Success,
    Error,
    Canceled,
    Timeout,
}
