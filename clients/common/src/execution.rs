//! UI-independent execution state. A cancellation request remains active until
//! the server publishes settlement; transport errors do not change this state.
use serde::Deserialize;
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecutionState {
    pub active: Option<Run>,
    pub last: Option<Run>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub run_id: String,
    pub options: crate::run_options::RunOptions,
    pub status: RunStatus,
    pub error: Option<String>,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    CancelRequested,
    Success,
    Error,
    Canceled,
    Timeout,
}
