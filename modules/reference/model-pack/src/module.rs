use std::{sync::Arc, time::Duration};

use futures_util::StreamExt;
use proteus_contracts::{
    contracts::{
        Model, ProcessModelDescriptor, ProcessModelEvent, ProcessModelInput, ProcessModelOutput,
        ProcessModelTerminal,
    },
    domain::ModelRef,
    model_standard::{ModelFailure, ModelStreamEvent},
    process_module::{
        ModelModule, ModelModuleHost, ModuleRegistry, ProcessModuleError, ProcessModuleResult,
    },
};
use serde_json::Value;

use crate::{
    adapters::{build_anthropic_messages_adapter, build_openai_responses_adapter},
    fake::FakeModelClient,
};

struct ProviderModule {
    runtime: tokio::runtime::Runtime,
    streaming: Arc<dyn Model>,
    complete: Arc<dyn Model>,
}

pub fn register_model(registry: &mut dyn ModuleRegistry, id: &str) -> ProcessModuleResult<()> {
    let implementation = registry
        .module_config()
        .get("implementation")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ProcessModuleError::new("reference model requires module_config.implementation")
        })?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(error)?;
    let (streaming, complete) = {
        let _guard = runtime.enter();
        (
            build(implementation, registry.module_config(), true)?,
            build(implementation, registry.module_config(), false)?,
        )
    };
    registry.register_model(
        id.to_owned(),
        Box::new(ProviderModule {
            runtime,
            streaming,
            complete,
        }),
    )
}

fn build(id: &str, config: &Value, stream: bool) -> ProcessModuleResult<Arc<dyn Model>> {
    let mut config = config
        .as_object()
        .cloned()
        .ok_or_else(|| ProcessModuleError::new("model module_config must be an object"))?;
    config.insert("stream".into(), stream.into());
    let config = Value::Object(config);
    match id {
        "openai" | "openai_compatible" => build_openai_responses_adapter(config).map_err(error),
        "anthropic" => build_anthropic_messages_adapter(config).map_err(error),
        "fake" => Ok(if stream {
            Arc::new(FakeModelClient::with_streaming(
                config.get("stream_delay_ms").and_then(Value::as_u64),
            ))
        } else {
            Arc::new(FakeModelClient::default())
        }),
        _ => Err(ProcessModuleError::new(format!(
            "unknown reference model {id}"
        ))),
    }
}

impl ModelModule for ProviderModule {
    fn describe(&self) -> ProcessModelDescriptor {
        // Reference capabilities are export-configured and do not depend on model names.
        let model = ModelRef::new("", "");
        ProcessModelDescriptor {
            adapter_id: self.streaming.id().into_owned(),
            capabilities: self.streaming.capabilities(&model),
            hosted_tools: self.streaming.provider_hosted_tools(&model),
        }
    }

    fn stream(
        &self,
        input: ProcessModelInput,
        host: &dyn ModelModuleHost,
    ) -> ProcessModuleResult<ProcessModelOutput> {
        self.runtime.block_on(async {
            let adapter = if input.stream { &self.streaming } else { &self.complete };
            let canceled = async {
                loop {
                    if host.is_cancelled() { break; }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            };
            tokio::pin!(canceled);
            let mut count = 0;
            let terminal = tokio::select! {
                biased;
                _ = &mut canceled => return Err(ProcessModuleError::new("model invocation canceled")),
                result = async {
                    let mut stream = match adapter.stream(input.request).await {
                        Ok(stream) => stream,
                        Err(err) => return Ok(ProcessModelTerminal::RequestError { failure: ModelFailure::from_error(&err) }),
                    };
                    while let Some(event) = stream.next().await {
                        match event {
                            Ok(ModelStreamEvent::Response { response }) => return Ok(ProcessModelTerminal::Response { response }),
                            Ok(ModelStreamEvent::Error { failure }) => return Ok(ProcessModelTerminal::StreamError { failure }),
                            Err(err) => return Ok(ProcessModelTerminal::RequestError { failure: ModelFailure::from_error(&err) }),
                            Ok(event) => {
                                host.emit(ProcessModelEvent { sequence: count, event })?;
                                count = count.checked_add(1).ok_or_else(|| ProcessModuleError::new("model event sequence overflow"))?;
                            }
                        }
                    }
                    Ok::<_, ProcessModuleError>(ProcessModelTerminal::RequestError {
                        failure: ModelFailure::other("model stream ended without a complete response"),
                    })
                } => result?,
            };
            Ok(ProcessModelOutput { event_count: count, terminal })
        })
    }
}

fn error(value: impl std::fmt::Display) -> ProcessModuleError {
    ProcessModuleError::new(value.to_string())
}
