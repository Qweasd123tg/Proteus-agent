//! Adapter: `CompactorObject` -> `Arc<dyn HistoryCompactor>`.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;

use proteus_contracts::{
    abi_stable::{
        sabi_trait::TD_Opaque,
        std_types::{RResult, RString},
    },
    plugin::{
        CompactorObject, PluginCompactionError, PluginCompactorHost, PluginCompactorHost_TO,
        PluginCompactorHostMut, PluginHistoryCompactor_TO,
    },
};
use tokio::{runtime::Handle, time::timeout};

use crate::{
    contracts::{
        CompactionHost, CompactionInput, CompactionOutput, HistoryCompactor, RuntimeContext,
    },
    model_standard::{CanonicalModelRequest, CanonicalModelResponse},
};

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
    async fn compact(
        &self,
        input: CompactionInput,
        host: Arc<dyn CompactionHost>,
    ) -> Result<CompactionOutput> {
        let input_json = serde_json::to_string(&input)
            .with_context(|| "plugin compactor: serialize CompactionInput failed")?;
        let inner = self.inner.clone();
        let handle = Handle::current();
        let output_json = tokio::task::spawn_blocking(move || {
            let mut host = CompactorHostBridge { host, handle };
            let mut host_to: PluginCompactorHostMut<'_> =
                PluginCompactorHost_TO::from_ptr(&mut host, TD_Opaque);
            match PluginHistoryCompactor_TO::compact_json(
                &*inner,
                RString::from(input_json),
                &mut host_to,
            ) {
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

struct CompactorHostBridge {
    host: Arc<dyn CompactionHost>,
    handle: Handle,
}

impl PluginCompactorHost for CompactorHostBridge {
    fn is_cancelled(&self) -> RResult<bool, PluginCompactionError> {
        RResult::ROk(self.host.is_cancelled())
    }

    fn complete_model_json(
        &self,
        request_json: RString,
    ) -> RResult<RString, PluginCompactionError> {
        let request: CanonicalModelRequest = match serde_json::from_str(request_json.as_str()) {
            Ok(request) => request,
            Err(error) => return RResult::RErr(PluginCompactionError::new(error.to_string())),
        };
        if self.host.is_cancelled() {
            return RResult::RErr(PluginCompactionError::new("turn canceled by client"));
        }
        let host = self.host.clone();
        match self
            .handle
            .block_on(async move { host.complete_model(request).await })
        {
            Ok(response) => match serde_json::to_string(&response) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => RResult::RErr(PluginCompactionError::new(error.to_string())),
            },
            Err(error) => RResult::RErr(PluginCompactionError::new(format!("{error:#}"))),
        }
    }
}

#[derive(Clone)]
pub struct RuntimeCompactionHost {
    ctx: RuntimeContext,
}

impl RuntimeCompactionHost {
    pub fn new(ctx: RuntimeContext) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl CompactionHost for RuntimeCompactionHost {
    fn is_cancelled(&self) -> bool {
        self.ctx.is_cancelled()
    }

    async fn complete_model(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse> {
        if self.ctx.is_cancelled() {
            anyhow::bail!("turn canceled by client");
        }
        let ctx = self.ctx.clone();
        let cancellation = ctx.cancellation.clone();
        tokio::select! {
            result = async move {
                if ctx.model_timeout_ms == 0 {
                    ctx.model.complete(request).await
                } else {
                    timeout(
                        Duration::from_millis(ctx.model_timeout_ms),
                        ctx.model.complete(request),
                    )
                    .await
                    .map_err(|_| anyhow!("model request timed out after {}ms", ctx.model_timeout_ms))?
                }
            } => result,
            _ = cancellation.cancelled() => Err(anyhow!("turn canceled by client")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use proteus_contracts::{
        abi_stable::{sabi_trait::TD_Opaque, std_types::RResult::ROk},
        domain::{AgentTask, ModelRef},
        model_standard::{
            CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart,
            FinishReason, MessageRole,
        },
        plugin::{
            PluginCompactionError, PluginCompactorHostMut, PluginHistoryCompactor,
            PluginHistoryCompactor_TO,
        },
    };

    struct StaticCompactor;
    impl PluginHistoryCompactor for StaticCompactor {
        fn compact_json(
            &self,
            input_json: RString,
            _host: &mut PluginCompactorHostMut<'_>,
        ) -> RResult<RString, PluginCompactionError> {
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
        fn compact_json(
            &self,
            _input_json: RString,
            _host: &mut PluginCompactorHostMut<'_>,
        ) -> RResult<RString, PluginCompactionError> {
            RResult::RErr(PluginCompactionError::new("compaction failed"))
        }
    }

    struct BrokenJsonCompactor;
    impl PluginHistoryCompactor for BrokenJsonCompactor {
        fn compact_json(
            &self,
            _input_json: RString,
            _host: &mut PluginCompactorHostMut<'_>,
        ) -> RResult<RString, PluginCompactionError> {
            ROk(RString::from("not json"))
        }
    }

    struct HostCallingCompactor;
    impl PluginHistoryCompactor for HostCallingCompactor {
        fn compact_json(
            &self,
            _input_json: RString,
            host: &mut PluginCompactorHostMut<'_>,
        ) -> RResult<RString, PluginCompactionError> {
            let request = CanonicalModelRequest::new(
                ModelRef::new("fake", "fake-summary-model"),
                vec![CanonicalMessage::text(MessageRole::User, "summarize")],
            );
            let response_json = match host
                .complete_model_json(RString::from(serde_json::to_string(&request).unwrap()))
            {
                RResult::ROk(json) => json,
                RResult::RErr(error) => return RResult::RErr(error),
            };
            let response: CanonicalModelResponse =
                serde_json::from_str(response_json.as_str()).unwrap();
            let summary = response
                .message
                .parts
                .iter()
                .find_map(|part| match part {
                    ContentPart::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .unwrap();
            let output = CompactionOutput::changed(
                vec![CanonicalMessage::text(MessageRole::User, summary.clone())],
                Some(summary),
            );
            ROk(serde_json::to_string(&output).unwrap().into())
        }
    }

    #[derive(Default)]
    struct RecordingCompactionHost {
        requests: Mutex<Vec<CanonicalModelRequest>>,
    }

    #[async_trait]
    impl CompactionHost for RecordingCompactionHost {
        async fn complete_model(
            &self,
            request: CanonicalModelRequest,
        ) -> Result<CanonicalModelResponse> {
            self.requests.lock().unwrap().push(request);
            Ok(CanonicalModelResponse::new(
                CanonicalMessage::text(MessageRole::Assistant, "model generated summary"),
                Vec::new(),
                FinishReason::Stop,
            ))
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

    fn host() -> Arc<dyn CompactionHost> {
        Arc::new(RecordingCompactionHost::default())
    }

    #[tokio::test]
    async fn plugin_success_round_trip() {
        let adapter = wrap(StaticCompactor);
        let output = adapter.compact(make_input(), host()).await.unwrap();
        assert!(output.changed);
        assert_eq!(output.messages.len(), 1);
        assert_eq!(output.summary.as_deref(), Some("1 messages"));
    }

    #[tokio::test]
    async fn plugin_rerror_propagates_as_anyhow() {
        let adapter = wrap(FailCompactor);
        let err = adapter.compact(make_input(), host()).await.unwrap_err();
        assert!(err.to_string().contains("compaction failed"), "{err}");
    }

    #[tokio::test]
    async fn invalid_json_propagates_as_anyhow() {
        let adapter = wrap(BrokenJsonCompactor);
        let err = adapter.compact(make_input(), host()).await.unwrap_err();
        assert!(
            err.to_string().contains("invalid CompactionOutput"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn plugin_can_call_model_through_compaction_host() {
        let adapter = wrap(HostCallingCompactor);
        let host = Arc::new(RecordingCompactionHost::default());

        let output = adapter.compact(make_input(), host.clone()).await.unwrap();

        assert!(output.changed);
        assert_eq!(output.summary.as_deref(), Some("model generated summary"));
        let requests = host.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].messages.len(), 1);
    }
}
