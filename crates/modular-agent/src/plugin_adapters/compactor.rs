//! Adapter: `CompactorObject` -> `Arc<dyn HistoryCompactor>`.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;

use agent_contracts::{
    abi_stable::std_types::{RResult, RString},
    plugin::{CompactorObject, PluginHistoryCompactor_TO},
};

use crate::contracts::{CompactionInput, CompactionOutput, HistoryCompactor};

pub struct PluginCompactorAdapter {
    inner: Arc<CompactorObject>,
}

impl PluginCompactorAdapter {
    pub fn new(inner: CompactorObject) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }
}

#[async_trait]
impl HistoryCompactor for PluginCompactorAdapter {
    async fn compact(&self, input: CompactionInput) -> Result<CompactionOutput> {
        let input_json = serde_json::to_string(&input)
            .with_context(|| "plugin compactor: serialize CompactionInput failed")?;
        let inner = self.inner.clone();
        let output_json = tokio::task::spawn_blocking(move || {
            match PluginHistoryCompactor_TO::compact_json(&*inner, RString::from(input_json)) {
                RResult::ROk(output) => Ok(output.into_string()),
                RResult::RErr(error) => Err(anyhow!("plugin compactor error: {}", error.message)),
            }
        })
        .await
        .map_err(|join_err| anyhow!("plugin compactor join error: {join_err}"))??;

        serde_json::from_str(&output_json)
            .with_context(|| "plugin compactor returned invalid CompactionOutput JSON")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_contracts::{
        abi_stable::{sabi_trait::TD_Opaque, std_types::RResult::ROk},
        domain::{AgentTask, ModelRef},
        model_standard::{CanonicalMessage, MessageRole},
        plugin::{PluginCompactionError, PluginHistoryCompactor, PluginHistoryCompactor_TO},
    };

    struct StaticCompactor;
    impl PluginHistoryCompactor for StaticCompactor {
        fn compact_json(&self, input_json: RString) -> RResult<RString, PluginCompactionError> {
            let input: CompactionInput = serde_json::from_str(input_json.as_str()).unwrap();
            let output = CompactionOutput::changed(
                vec![CanonicalMessage::text(MessageRole::System, "summary")],
                Some(format!("{} messages", input.messages.len())),
            );
            ROk(serde_json::to_string(&output).unwrap().into())
        }
    }

    struct FailCompactor;
    impl PluginHistoryCompactor for FailCompactor {
        fn compact_json(&self, _input_json: RString) -> RResult<RString, PluginCompactionError> {
            RResult::RErr(PluginCompactionError::new("compaction failed"))
        }
    }

    struct BrokenJsonCompactor;
    impl PluginHistoryCompactor for BrokenJsonCompactor {
        fn compact_json(&self, _input_json: RString) -> RResult<RString, PluginCompactionError> {
            ROk(RString::from("not json"))
        }
    }

    fn wrap(compactor: impl PluginHistoryCompactor + 'static) -> PluginCompactorAdapter {
        let obj = PluginHistoryCompactor_TO::from_value(compactor, TD_Opaque);
        PluginCompactorAdapter::new(obj)
    }

    fn make_input() -> CompactionInput {
        CompactionInput::new(
            AgentTask::new("task", std::path::PathBuf::from("/tmp")),
            ModelRef::new("fake", "fake-tool-model"),
            vec![CanonicalMessage::text(MessageRole::User, "hello")],
        )
        .with_reason("test")
    }

    #[tokio::test]
    async fn plugin_success_round_trip() {
        let adapter = wrap(StaticCompactor);
        let output = adapter.compact(make_input()).await.unwrap();
        assert!(output.changed);
        assert_eq!(output.messages.len(), 1);
        assert_eq!(output.summary.as_deref(), Some("1 messages"));
    }

    #[tokio::test]
    async fn plugin_rerror_propagates_as_anyhow() {
        let adapter = wrap(FailCompactor);
        let err = adapter.compact(make_input()).await.unwrap_err();
        assert!(err.to_string().contains("compaction failed"), "{err}");
    }

    #[tokio::test]
    async fn invalid_json_propagates_as_anyhow() {
        let adapter = wrap(BrokenJsonCompactor);
        let err = adapter.compact(make_input()).await.unwrap_err();
        assert!(err.to_string().contains("invalid CompactionOutput"), "{err}");
    }
}
