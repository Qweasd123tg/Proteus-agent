use std::path::PathBuf;

use crate::domain::{PolicyDecision, ToolCall, ToolSpec};

#[derive(Debug, Clone)]
pub struct PolicyContext {
    pub cwd: PathBuf,
    pub tool_spec: Option<ToolSpec>,
}

pub trait ApprovalPolicy: Send + Sync {
    fn evaluate(&self, call: &ToolCall, ctx: &PolicyContext) -> PolicyDecision;
}
