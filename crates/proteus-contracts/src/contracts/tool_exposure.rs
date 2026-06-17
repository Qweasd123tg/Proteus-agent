use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::{AgentTask, ToolSpec};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl ToolExposureRequest {
    pub fn new(task: AgentTask) -> Self {
        Self {
            cwd: task.cwd.clone(),
            task,
            query: None,
            max_tools: None,
            reason: None,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolExposureInput {
    pub request: ToolExposureRequest,
    #[serde(default)]
    pub candidates: Vec<ToolSpec>,
    #[serde(default)]
    pub config: serde_json::Value,
}

impl ToolExposureInput {
    pub fn new(request: ToolExposureRequest, candidates: Vec<ToolSpec>) -> Self {
        Self {
            request,
            candidates,
            config: serde_json::Value::Null,
        }
    }

    pub fn with_config(mut self, config: serde_json::Value) -> Self {
        self.config = config;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolExposureOutput {
    #[serde(default)]
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
}

#[async_trait]
pub trait ToolExposure: Send + Sync {
    async fn select(&self, input: ToolExposureInput) -> Result<ToolExposureOutput>;
}
