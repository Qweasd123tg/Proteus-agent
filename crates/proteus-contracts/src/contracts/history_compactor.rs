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
    /// Сырой потолок контекстного окна модели. Компактор применяет к нему
    /// `trigger_fraction` из конфига. `None` — если окно неизвестно.
    #[serde(default)]
    pub window_tokens: Option<u32>,
    /// module-config компактора (`module_config.compactor.<id>`), который
    /// хост прокидывает в плагин. Содержит, в частности, порог автокомпакта.
    #[serde(default)]
    pub config: serde_json::Value,
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
            window_tokens: None,
            config: serde_json::Value::Null,
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

    pub fn with_window_tokens(mut self, window_tokens: Option<u32>) -> Self {
        self.window_tokens = window_tokens;
        self
    }

    pub fn with_config(mut self, config: serde_json::Value) -> Self {
        self.config = config;
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
            metadata_usize(&metadata, "input_messages").unwrap_or(input.messages.len());
        let output_messages =
            metadata_usize(&metadata, "output_messages").unwrap_or(output.messages.len());
        Self {
            changed: output.changed,
            reason: input.reason.clone(),
            input_messages,
            output_messages,
            original_token_estimate: metadata_u32(&metadata, "original_token_estimate")
                .or(input.token_estimate),
            output_token_estimate: metadata_u32(&metadata, "output_token_estimate")
                .or(output.token_estimate),
            trigger_tokens: metadata_u32(&metadata, "trigger_tokens"),
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{domain::AgentTask, model_standard::MessageRole};

    fn sample_input() -> CompactionInput {
        CompactionInput::new(
            AgentTask::new("continue", std::path::PathBuf::from("/repo")),
            ModelRef::new("fake", "model"),
            vec![CanonicalMessage::text(MessageRole::User, "hello")],
        )
        .with_max_tokens(Some(100))
    }

    #[test]
    fn report_does_not_invent_trigger_from_legacy_input_max_tokens() {
        let input = sample_input();
        let output = CompactionOutput::unchanged(input.messages.clone());

        let report = HistoryCompactionReport::from_compaction_output(&input, &output);

        assert_eq!(report.trigger_tokens, None);
    }

    #[test]
    fn report_uses_trigger_from_compactor_metadata() {
        let input = sample_input();
        let mut output = CompactionOutput::unchanged(input.messages.clone());
        output.metadata = json!({ "trigger_tokens": 80 });

        let report = HistoryCompactionReport::from_compaction_output(&input, &output);

        assert_eq!(report.trigger_tokens, Some(80));
    }
}
