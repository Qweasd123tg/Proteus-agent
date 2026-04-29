use std::path::PathBuf;

use crate::domain::{PolicyDecision, ToolCall, ToolSpec};

#[derive(Debug, Clone)]
pub struct PolicyContext {
    pub cwd: PathBuf,
    pub tool_spec: Option<ToolSpec>,
}

#[derive(Debug, Clone)]
pub struct PolicyVisibilityContext {
    pub cwd: PathBuf,
    pub tool_spec: ToolSpec,
}

pub trait ApprovalPolicy: Send + Sync {
    fn evaluate(&self, call: &ToolCall, ctx: &PolicyContext) -> PolicyDecision;

    fn evaluate_visibility(&self, ctx: &PolicyVisibilityContext) -> PolicyDecision;
}
