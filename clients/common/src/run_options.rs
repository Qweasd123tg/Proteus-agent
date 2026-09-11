use serde::{Deserialize, Serialize};

/// Public per-execution parameters; intent semantics belong to the workflow.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOptions {
    pub intent: Option<String>,
    pub permission_mode: Option<String>,
}
