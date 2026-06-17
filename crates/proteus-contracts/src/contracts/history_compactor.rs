use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{AgentTask, HistoryCompactionReport, ModelRef},
    model_standard::{CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CompactionInput {
    pub task: AgentTask,
    pub model_ref: ModelRef,
    #[serde(default)]
    pub messages: Vec<CanonicalMessage>,
    #[serde(default)]
    pub token_estimate: Option<u32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub reason: Option<String>,
}

impl CompactionInput {
    pub fn new(task: AgentTask, model_ref: ModelRef, messages: Vec<CanonicalMessage>) -> Self {
        Self {
            task,
            model_ref,
            messages,
            token_estimate: None,
            max_tokens: None,
            reason: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_token_estimate(mut self, token_estimate: Option<u32>) -> Self {
        self.token_estimate = token_estimate;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: Option<u32>) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CompactionOutput {
    #[serde(default)]
    pub messages: Vec<CanonicalMessage>,
    #[serde(default)]
    pub changed: bool,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub token_estimate: Option<u32>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl CompactionOutput {
    pub fn changed(messages: Vec<CanonicalMessage>, summary: impl Into<Option<String>>) -> Self {
        Self {
            messages,
            changed: true,
            summary: summary.into(),
            token_estimate: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn unchanged(messages: Vec<CanonicalMessage>) -> Self {
        Self {
            messages,
            changed: false,
            summary: None,
            token_estimate: None,
            metadata: serde_json::Value::Null,
        }
    }
}

impl HistoryCompactionReport {
    pub fn from_compaction_output(input: &CompactionInput, output: &CompactionOutput) -> Self {
        let metadata = output.metadata.clone();
        let input_messages =
            metadata_usize(&metadata, "input_messages").unwrap_or_else(|| input.messages.len());
        let output_messages =
            metadata_usize(&metadata, "output_messages").unwrap_or_else(|| output.messages.len());
        Self {
            changed: output.changed,
            reason: input.reason.clone(),
            input_messages,
            output_messages,
            original_token_estimate: metadata_u32(&metadata, "original_token_estimate")
                .or(input.token_estimate),
            output_token_estimate: metadata_u32(&metadata, "output_token_estimate")
                .or(output.token_estimate),
            trigger_tokens: metadata_u32(&metadata, "trigger_tokens").or(input.max_tokens),
            summary_source: metadata_string(&metadata, "summary_source"),
            skipped_reason: metadata_string(&metadata, "skipped_reason"),
            summary: output.summary.clone(),
            metadata,
        }
    }
}

fn metadata_u32(metadata: &serde_json::Value, key: &str) -> Option<u32> {
    metadata
        .get(key)?
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
}

fn metadata_usize(metadata: &serde_json::Value, key: &str) -> Option<usize> {
    metadata
        .get(key)?
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
}

fn metadata_string(metadata: &serde_json::Value, key: &str) -> Option<String> {
    metadata.get(key)?.as_str().map(ToOwned::to_owned)
}

#[async_trait]
pub trait CompactionHost: Send + Sync {
    fn is_cancelled(&self) -> bool {
        false
    }

    async fn complete_model(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse>;
}

#[async_trait]
pub trait HistoryCompactor: Send + Sync {
    async fn compact(
        &self,
        input: CompactionInput,
        host: std::sync::Arc<dyn CompactionHost>,
    ) -> Result<CompactionOutput>;
}
