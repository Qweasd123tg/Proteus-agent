use std::{borrow::Cow, path::Path, pin::Pin, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;
use futures_util::Stream;
use proteus_module_protocol::{
    ProcessModuleRpcError,
    v3::{
        AsyncHostRequestDispatcher, CancelCause, ComponentHostRequest, HostRequestFuture,
        InvocationCancelHandle,
    },
};
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};

use super::{ProcessExportClient, ProcessExportConfig};
use crate::{
    contracts::{
        MODEL_HOST_EMIT_METHOD, Model, PROCESS_MODEL_CONTRACT_VERSION,
        PROCESS_MODEL_DESCRIBE_METHOD, PROCESS_MODEL_STREAM_METHOD, ProcessModelDescriptor,
        ProcessModelEvent, ProcessModelInput, ProcessModelOutput, ProcessModelTerminal,
    },
    domain::{ModelRef, ToolSpec},
    model_standard::{CanonicalModelRequest, ModelCapabilities, ModelStreamEvent},
};

pub struct ProcessModel {
    client: Arc<ProcessExportClient>,
    descriptor: ProcessModelDescriptor,
    stream: bool,
}

impl ProcessModel {
    pub fn new(
        config: ProcessExportConfig,
        cwd: &Path,
        stream: bool,
        timeout_ms: u64,
    ) -> Result<Self> {
        let client = Arc::new(ProcessExportClient::connect(
            "model",
            PROCESS_MODEL_CONTRACT_VERSION,
            config,
            cwd,
            timeout_ms,
        )?);
        let descriptor: ProcessModelDescriptor =
            client.invoke_bootstrap(PROCESS_MODEL_DESCRIBE_METHOD, &Value::Null)?;
        if descriptor.adapter_id.trim().is_empty() {
            client.reset();
            bail!("model describe returned an empty adapter_id");
        }
        Ok(Self {
            client,
            descriptor,
            stream,
        })
    }
}

// A stream can be dropped before its first poll, including by BoundModel cancellation.
struct CancelOnDrop(Option<InvocationCancelHandle>);

impl CancelOnDrop {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            let _ = cancel.cancel(CancelCause::User);
        }
    }
}

struct EventSink {
    next: Arc<Mutex<u64>>,
    events: mpsc::Sender<Result<ModelStreamEvent>>,
}

impl AsyncHostRequestDispatcher for EventSink {
    fn dispatch(&self, request: ComponentHostRequest) -> HostRequestFuture {
        let events = self.events.clone();
        // Parse synchronously, but serialize validation and acknowledged delivery below.
        let parsed = if request.method == MODEL_HOST_EMIT_METHOD {
            serde_json::from_value::<ProcessModelEvent>(request.params)
                .map_err(|e| ProcessModuleRpcError::new(-32602, e.to_string()))
        } else {
            Err(ProcessModuleRpcError::new(
                -32601,
                "unsupported model host method",
            ))
        };
        // The per-invocation counter is shared by all concurrent callbacks.
        let next = self.next.clone();
        Box::pin(async move {
            let mut next = next.lock().await;
            let input = match parsed.and_then(|input| {
                if input.sequence != *next
                    || matches!(
                        input.event,
                        ModelStreamEvent::Response { .. } | ModelStreamEvent::Error { .. }
                    )
                {
                    Err(ProcessModuleRpcError::new(
                        -32602,
                        "invalid model event sequence or terminal event in emit",
                    ))
                } else {
                    Ok(input)
                }
            }) {
                Ok(input) => input,
                Err(error) => {
                    // A worker cannot turn a rejected callback into success by ignoring its RPC error.
                    let _ = events
                        .send(Err(anyhow::anyhow!(
                            "invalid model stream: {}",
                            error.message
                        )))
                        .await;
                    return Err(error);
                }
            };
            events
                .send(Ok(input.event))
                .await
                .map_err(|_| ProcessModuleRpcError::new(-32000, "model stream consumer closed"))?;
            *next = next.checked_add(1).ok_or_else(|| {
                ProcessModuleRpcError::new(-32602, "model event sequence overflow")
            })?;
            Ok(Value::Null)
        })
    }
}

#[async_trait]
impl Model for ProcessModel {
    fn id(&self) -> Cow<'static, str> {
        Cow::Owned(self.descriptor.adapter_id.clone())
    }
    fn capabilities(&self, _model: &ModelRef) -> ModelCapabilities {
        self.descriptor.capabilities.clone()
    }
    fn provider_hosted_tools(&self, _model: &ModelRef) -> Vec<ToolSpec> {
        self.descriptor.hosted_tools.clone()
    }

    async fn stream(
        &self,
        request: CanonicalModelRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ModelStreamEvent>> + Send>>> {
        let (tx, mut rx) = mpsc::channel(1);
        let sink = Arc::new(EventSink {
            next: Arc::new(Mutex::new(0)),
            events: tx,
        });
        let client = self.client.clone();
        let mut handle = client
            .start(
                PROCESS_MODEL_STREAM_METHOD,
                serde_json::to_value(ProcessModelInput {
                    request,
                    stream: self.stream,
                })?,
                sink,
            )
            .await?;
        let mut guard = CancelOnDrop(Some(handle.cancel_handle()));
        Ok(Box::pin(async_stream::try_stream! {
            let mut terminal = Box::pin(handle.result());
            let mut received = 0u64;
            let terminal = loop {
                let next = tokio::select! {
                    event = rx.recv(), if !rx.is_closed() || !rx.is_empty() => Ok(event),
                    result = &mut terminal => Err(result),
                };
                match next {
                    Ok(Some(event)) => {
                        if event.is_err() { client.reset(); }
                        received += 1;
                        yield event?;
                    }
                    Ok(None) => {},
                    Err(result) => break result,
                }
            };
            // Callback acknowledgement can precede consumer delivery of the last queued item.
            while let Ok(event) = rx.try_recv() {
                if event.is_err() { client.reset(); }
                received += 1; yield event?;
            }
            guard.disarm();
            let output: ProcessModelOutput = client.decode(PROCESS_MODEL_STREAM_METHOD, terminal?).map_err(model_invocation_error)?;
            if output.event_count != received {
                client.reset();
                Err(anyhow::anyhow!("model terminal event_count does not match emitted events"))?;
            }
            match output.terminal {
                ProcessModelTerminal::Response { response } => yield ModelStreamEvent::Response { response },
                ProcessModelTerminal::StreamError { failure } => yield ModelStreamEvent::Error { failure },
                ProcessModelTerminal::RequestError { failure } => Err(anyhow::Error::new(failure))?,
            }
        }))
    }
}

fn model_invocation_error(error: anyhow::Error) -> anyhow::Error {
    if let Some(invocation) = error.downcast_ref::<super::ProcessInvocationError>()
        && matches!(
            invocation.failure(),
            super::ProcessInvocationFailure::Canceled
        )
    {
        return crate::model_standard::ModelFailure::new(
            crate::model_standard::ModelFailureKind::Interrupted,
            error.to_string(),
        )
        .into();
    }
    error
}
