use crate::domain::PermissionMode;
use serde::{Deserialize, Serialize};

/// Parameters of one new execution, never mutations of session defaults.
/// Intent names and their semantics belong to the selected workflow.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOptions {
    pub intent: Option<String>,
    pub permission_mode: Option<PermissionMode>,
}
