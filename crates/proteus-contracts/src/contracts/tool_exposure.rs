use std::path::PathBuf;

use anyhow::{Result, ensure};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::{AgentTask, ToolSpec};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ToolExposureRequest {
    pub task: AgentTask,
    pub cwd: PathBuf,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub max_tools: Option<usize>,
    #[serde(default)]
    pub reason: Option<String>,
    /// Фаза workflow ("plan"/"execute"/"review"/...), если workflow фазовый.
    /// Selector может использовать её для phase-aware отбора tools.
    #[serde(default)]
    pub phase: Option<String>,
}

impl ToolExposureRequest {
    pub fn new(task: AgentTask) -> Self {
        Self {
            cwd: task.cwd.clone(),
            task,
            query: None,
            max_tools: None,
            reason: None,
            phase: None,
        }
    }

    pub fn with_query(mut self, query: impl Into<String>) -> Self {
        self.query = Some(query.into());
        self
    }

    pub fn with_max_tools(mut self, max_tools: usize) -> Self {
        self.max_tools = Some(max_tools);
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_phase(mut self, phase: impl Into<String>) -> Self {
        self.phase = Some(phase.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ToolExposureInput {
    pub request: ToolExposureRequest,
    pub candidates: Vec<ToolSpec>,
}

impl ToolExposureInput {
    pub fn new(request: ToolExposureRequest, candidates: Vec<ToolSpec>) -> Self {
        Self {
            request,
            candidates,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ToolExposureOutput {
    pub tools: Vec<ToolSpec>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl ToolExposureOutput {
    pub fn new(tools: Vec<ToolSpec>) -> Self {
        Self {
            tools,
            metadata: serde_json::Value::Null,
        }
    }

    /// Selection can reorder or omit candidates, but cannot redefine tools.
    pub fn validate_against(&self, candidates: &[ToolSpec]) -> Result<()> {
        let mut names = std::collections::HashSet::new();
        for tool in &self.tools {
            ensure!(
                names.insert(&tool.name),
                "duplicate selected tool: {}",
                tool.name
            );
            ensure!(
                candidates.iter().any(|candidate| candidate == tool),
                "tool exposure changed or invented registered tool '{}'",
                tool.name
            );
        }
        Ok(())
    }
}

#[async_trait]
pub trait ToolExposure: Send + Sync {
    async fn select(&self, input: ToolExposureInput) -> Result<ToolExposureOutput>;
}
