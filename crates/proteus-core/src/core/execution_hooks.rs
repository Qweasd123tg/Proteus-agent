//! Ordered host-owned hook execution and durable accepted boundary facts.
use crate::{
    contracts::{
        CancellationToken, ExecutionHooks, ExecutionRecorder, ExecutionScope, HookEvent,
        HookHandler, HookInput, HookStep, HookStepOutcome, HookTrace, apply_hook_response,
    },
    core::ModelService,
};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::sync::Arc;

pub struct RuntimeHookChain {
    handlers: Vec<(String, Arc<dyn HookHandler>)>,
    scope: ExecutionScope,
    recorder: Arc<dyn ExecutionRecorder>,
    model_service: Option<Arc<ModelService>>,
}
impl RuntimeHookChain {
    pub fn new(
        handlers: Vec<(String, Arc<dyn HookHandler>)>,
        scope: ExecutionScope,
        recorder: Arc<dyn ExecutionRecorder>,
        model_service: Option<Arc<ModelService>>,
    ) -> Self {
        Self {
            handlers,
            scope,
            recorder,
            model_service,
        }
    }
}
#[async_trait]
impl ExecutionHooks for RuntimeHookChain {
    fn is_active(&self) -> bool {
        !self.handlers.is_empty()
    }
    async fn apply(&self, input: HookInput) -> Result<HookEvent> {
        if input.attribution.execution_id != self.scope.execution_id {
            anyhow::bail!("hook attribution conflicts with bound execution scope");
        }
        if self.handlers.is_empty() {
            return Ok(input.event);
        }
        let notification = matches!(
            &input.event,
            HookEvent::TurnStarted { .. } | HookEvent::TurnSettled { .. }
        );
        let cleanup = matches!(&input.event, HookEvent::TurnSettled { .. });
        let cancellation = if cleanup {
            CancellationToken::new()
        } else {
            self.scope.cancellation.child_token()
        };
        let mut trace = HookTrace {
            input: input.clone(),
            steps: vec![],
            output: None,
        };
        let mut event = input.event.clone();
        for (module_id, handler) in &self.handlers {
            let mut invocation = input.clone();
            invocation.event = event.clone();
            let outcome = if cancellation.is_cancelled() {
                Err(anyhow!("hook execution canceled"))
            } else {
                let handler_cancellation = cancellation.child_token();
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        handler_cancellation.cancel();
                        Err(anyhow!("hook execution canceled"))
                    },
                    result = handler.invoke(invocation, handler_cancellation.clone()) => result,
                }
            };
            let validated = outcome.and_then(|response| {
                // Parts removed by an earlier contribution remain immutable at this boundary.
                if matches!(&input.event, HookEvent::BeforeModel { .. }) {
                    apply_hook_response(&input.event, &response)?;
                }
                let mut next = apply_hook_response(&event, &response)?;
                if let HookEvent::BeforeModel { request, .. } = &mut next {
                    *request = self
                        .model_service
                        .as_ref()
                        .ok_or_else(|| anyhow!("no model is configured"))?
                        .prepare_request(request.clone())?;
                }
                Ok((response, next))
            });
            match validated {
                Ok((response, next)) => {
                    trace.steps.push(HookStep {
                        module_id: module_id.clone(),
                        outcome: HookStepOutcome::Accepted { response },
                    });
                    event = next;
                    if matches!(
                        &event,
                        HookEvent::BeforeTool {
                            blocked: Some(_),
                            ..
                        } | HookEvent::BeforeStop {
                            continuation: Some(_),
                            ..
                        }
                    ) {
                        break;
                    }
                }
                Err(error) => {
                    // Replay reproduces this complete canonical failure without rebuilding
                    // process/adapter error sources.
                    let error = anyhow!(format!("{error:#}"));
                    trace.steps.push(HookStep {
                        module_id: module_id.clone(),
                        outcome: HookStepOutcome::Failed {
                            message: error.to_string(),
                        },
                    });
                    if !notification {
                        self.recorder.hook_recorded(&trace).await?;
                        return Err(error);
                    }
                }
            }
        }
        trace.output = Some(event.clone());
        self.recorder.hook_recorded(&trace).await?;
        Ok(event)
    }
}

#[cfg(test)]
#[path = "execution_hooks_tests.rs"]
mod tests;
