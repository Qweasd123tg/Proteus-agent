use std::path::PathBuf;

use crate::domain::{PolicyDecision, ToolCall, ToolSpec};

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PolicyContext {
    pub cwd: PathBuf,
    pub tool_spec: Option<ToolSpec>,
}

impl PolicyContext {
    pub fn new(cwd: PathBuf, tool_spec: Option<ToolSpec>) -> Self {
        Self { cwd, tool_spec }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PolicyVisibilityContext {
    pub cwd: PathBuf,
    pub tool_spec: ToolSpec,
}

impl PolicyVisibilityContext {
    pub fn new(cwd: PathBuf, tool_spec: ToolSpec) -> Self {
        Self { cwd, tool_spec }
    }
}

pub trait ApprovalPolicy: Send + Sync {
    fn evaluate(&self, call: &ToolCall, ctx: &PolicyContext) -> PolicyDecision;

    fn evaluate_visibility(&self, ctx: &PolicyVisibilityContext) -> PolicyDecision;
}
