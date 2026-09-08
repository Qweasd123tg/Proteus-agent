use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{AgentTask, CompactionUserMessageReplacement, HistoryCompactionReport},
    model_standard::{CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse},
};

pub const PROCESS_COMPACTOR_CONTRACT_VERSION: &str = "v7";
pub const PROCESS_COMPACTOR_METHOD: &str = "compact";
pub const COMPACTOR_HOST_COMPLETE_MODEL_METHOD: &str = "host.model.complete";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessCompactorCompleteModelInput {
    pub request: CanonicalModelRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CompactionInput {
    pub task: AgentTask,
    /// Full pending request. The selected compactor owns how it reuses its
    /// instructions, model controls, cache settings and conversation.
    pub request: CanonicalModelRequest,
    pub token_estimate: Option<u32>,
    /// Сырой потолок контекстного окна модели. Способ использования определяет
    /// выбранная стратегия компактора. `None` — если окно неизвестно.
    pub window_tokens: Option<u32>,
    /// module-config компактора (`module_config.compactor.<id>`), который
    /// host передаёт выбранному process module.
    pub config: serde_json::Value,
    pub reason: Option<String>,
}

impl CompactionInput {
    pub fn new(task: AgentTask, request: CanonicalModelRequest) -> Self {
        Self {
            task,
            request,
            token_estimate: None,
            window_tokens: None,
            config: serde_json::json!({}),
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
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CompactionOutput {
    pub messages: Vec<CanonicalMessage>,
    /// Explicit replacements of accepted user inputs. Contents must never be
    /// rewritten under an existing message identity.
    pub user_message_replacements: Vec<CompactionUserMessageReplacement>,
    pub changed: bool,
    pub summary: Option<String>,
    pub token_estimate: Option<u32>,
    /// Module's estimate for the original input, if it computed one itself.
    /// Otherwise the report retains the estimate supplied in CompactionInput.
    pub original_token_estimate: Option<u32>,
    /// Token-based trigger, if applicable to this compactor's strategy.
    pub trigger_tokens: Option<u32>,
    /// Descriptive module-defined labels; consumers must not dispatch on them.
    pub summary_source: Option<String>,
    pub skipped_reason: Option<String>,
    /// Opaque diagnostics. Never overrides the typed result or message counts.
    pub metadata: serde_json::Value,
}

impl CompactionOutput {
    pub fn changed(messages: Vec<CanonicalMessage>, summary: impl Into<Option<String>>) -> Self {
        Self {
            messages,
            changed: true,
            user_message_replacements: Vec::new(),
            summary: summary.into(),
            token_estimate: None,
            original_token_estimate: None,
            trigger_tokens: None,
            summary_source: None,
            skipped_reason: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn unchanged(messages: Vec<CanonicalMessage>) -> Self {
        Self {
            messages,
            changed: false,
            user_message_replacements: Vec::new(),
            summary: None,
            token_estimate: None,
            original_token_estimate: None,
            trigger_tokens: None,
            summary_source: None,
            skipped_reason: None,
            metadata: serde_json::Value::Null,
        }
    }
}

/// Строгий result метода `compact` в process-module protocol.
/// Отдельный envelope не позволяет молча принять старую bare-форму
/// `CompactionOutput` при изменении draft wire contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessCompactionResponse {
    pub output: CompactionOutput,
}

impl ProcessCompactionResponse {
    pub fn new(output: CompactionOutput) -> Self {
        Self { output }
    }
}

impl HistoryCompactionReport {
    pub fn from_compaction_output(input: &CompactionInput, output: &CompactionOutput) -> Self {
        Self {
            changed: output.changed,
            user_message_replacements: output.user_message_replacements.clone(),
            reason: input.reason.clone(),
            input_messages: input.request.messages.len(),
            output_messages: output.messages.len(),
            original_token_estimate: output.original_token_estimate.or(input.token_estimate),
            output_token_estimate: output.token_estimate,
            trigger_tokens: output.trigger_tokens,
            summary_source: output.summary_source.clone(),
            skipped_reason: output.skipped_reason.clone(),
            summary: output.summary.clone(),
            metadata: output.metadata.clone(),
        }
    }
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
    use crate::{
        domain::{AgentTask, ModelRef},
        model_standard::MessageRole,
    };

    fn sample_input() -> CompactionInput {
        CompactionInput::new(
            AgentTask::new("continue", std::path::PathBuf::from("/repo")),
            crate::model_standard::CanonicalModelRequest::new(
                ModelRef::new("fake", "model"),
                vec![CanonicalMessage::text(MessageRole::User, "hello")],
            ),
        )
    }

    #[test]
    fn compaction_input_preserves_the_pending_request_and_rejects_old_shape() {
        let mut input = sample_input();
        input
            .request
            .instructions
            .push(crate::model_standard::InstructionBlock::new(
                crate::model_standard::InstructionKind::System,
                "original instructions",
                100,
            ));
        input.request.cache.routing_key = Some("same-session".to_owned());
        input.request.limits.max_output_tokens = Some(12_345);
        let value = serde_json::to_value(&input).unwrap();
        let decoded: CompactionInput = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded.request, input.request);

        let mut old = value;
        let fields = old.as_object_mut().unwrap();
        let request = fields.remove("request").unwrap();
        fields.insert("model_ref".to_owned(), request["model"].clone());
        fields.insert("messages".to_owned(), request["messages"].clone());
        assert!(serde_json::from_value::<CompactionInput>(old).is_err());
    }

    #[test]
    fn report_does_not_invent_trigger_for_non_token_based_compactor() {
        let input = sample_input();
        let output = CompactionOutput::unchanged(input.request.messages.clone());

        let report = HistoryCompactionReport::from_compaction_output(&input, &output);

        assert_eq!(report.trigger_tokens, None);
    }

    #[test]
    fn report_uses_typed_compactor_diagnostics() {
        let input = sample_input();
        let mut output = CompactionOutput::unchanged(input.request.messages.clone());
        output.trigger_tokens = Some(80);
        output.original_token_estimate = Some(96);
        output.summary_source = Some("custom-strategy".to_owned());
        output.skipped_reason = Some("custom-reason".to_owned());

        let report = HistoryCompactionReport::from_compaction_output(&input, &output);

        assert_eq!(report.trigger_tokens, Some(80));
        assert_eq!(report.original_token_estimate, Some(96));
        assert_eq!(report.summary_source.as_deref(), Some("custom-strategy"));
        assert_eq!(report.skipped_reason.as_deref(), Some("custom-reason"));
    }

    #[test]
    fn report_metadata_cannot_override_canonical_counts_and_estimates() {
        let input = sample_input().with_token_estimate(Some(64));
        let mut output = CompactionOutput::unchanged(input.request.messages.clone());
        output.token_estimate = Some(12);
        output.metadata = json!({
            "input_messages": 999, "output_messages": 999,
            "original_token_estimate": 999, "output_token_estimate": 999,
            "trigger_tokens": 999, "summary_source": "private-label", "skipped_reason": "private-label"
        });
        let report = HistoryCompactionReport::from_compaction_output(&input, &output);
        assert_eq!(report.input_messages, input.request.messages.len());
        assert_eq!(report.output_messages, output.messages.len());
        assert_eq!(report.original_token_estimate, Some(64));
        assert_eq!(report.output_token_estimate, Some(12));
        assert_eq!(report.trigger_tokens, None);
        assert_eq!(report.summary_source, None);
        assert_eq!(report.skipped_reason, None);
        assert_eq!(report.metadata, output.metadata);
    }

    #[test]
    fn process_compaction_response_rejects_malformed_typed_diagnostics() {
        let output = CompactionOutput::unchanged(sample_input().request.messages);
        let valid = serde_json::to_value(ProcessCompactionResponse::new(output)).unwrap();
        for (field, value) in [
            ("original_token_estimate", json!(-1)),
            ("trigger_tokens", json!(u64::from(u32::MAX) + 1)),
            ("trigger_tokens", json!("80")),
            ("summary_source", json!([])),
            ("skipped_reason", json!(false)),
        ] {
            let mut invalid = valid.clone();
            invalid["output"][field] = value;
            serde_json::from_value::<ProcessCompactionResponse>(invalid)
                .expect_err("typed diagnostics must be validated at the process boundary");
        }
        serde_json::from_value::<ProcessCompactionResponse>(valid)
            .expect("a strategy may leave inapplicable diagnostics null");
    }

    #[test]
    fn process_compaction_response_rejects_bare_output_and_unknown_fields() {
        let input = sample_input();
        let output = CompactionOutput::unchanged(input.request.messages);
        let bare = serde_json::to_value(&output).expect("bare output value");
        serde_json::from_value::<ProcessCompactionResponse>(bare)
            .expect_err("bare CompactionOutput must not be accepted");

        let mut envelope =
            serde_json::to_value(ProcessCompactionResponse::new(output)).expect("envelope value");
        envelope
            .as_object_mut()
            .expect("response object")
            .insert("legacy_output".to_owned(), serde_json::Value::Null);
        serde_json::from_value::<ProcessCompactionResponse>(envelope)
            .expect_err("unknown response fields must be rejected");
    }
}
