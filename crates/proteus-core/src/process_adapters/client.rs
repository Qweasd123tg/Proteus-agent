use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use proteus_module_protocol::v3::{
    AsyncHostRequestDispatcher, CancelCause, ComponentBroker, InvocationTerminal,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::time::{MissedTickBehavior, interval};

use super::ProcessExportConfig;

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Typed view of one export on a shared multiplexed process component broker.
pub struct ProcessExportClient {
    component_id: String,
    module_id: String,
    target: proteus_contracts::contracts::ProcessComponentExportRef,
    timeout: Duration,
    broker: Arc<ComponentBroker>,
}

impl ProcessExportClient {
    pub fn connect(
        slot: &str,
        contract_version: &str,
        config: ProcessExportConfig,
        workspace: &Path,
        default_timeout_ms: u64,
    ) -> Result<Self> {
        config.validate_for(slot, default_timeout_ms)?;
        if config.binding().contract_version != contract_version {
            bail!(
                "component {:?} export {}/{} uses contract {:?}, expected {:?}",
                config.component_id(),
                config.slot(),
                config.module_id(),
                config.binding().contract_version,
                contract_version
            );
        }
        let timeout = config.timeout(default_timeout_ms);
        let component_id = config.component_id().to_owned();
        let module_id = config.module_id().to_owned();
        let target = config.binding().export_ref();
        let broker = config.connect(workspace)?;
        Ok(Self {
            component_id,
            module_id,
            target,
            timeout,
            broker,
        })
    }

    pub async fn invoke<P, R>(&self, method: &str, params: &P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value = self.encode(method, params)?;
        let terminal = self
            .broker
            .invoke(&self.target, method, value, self.timeout)
            .await
            .with_context(|| self.invocation_context(method))?;
        self.decode(method, terminal)
    }

    pub async fn invoke_with_dispatcher<P, R>(
        &self,
        method: &str,
        params: &P,
        dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
    ) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value = self.encode(method, params)?;
        let terminal = self
            .broker
            .invoke_with_dispatcher(&self.target, method, value, self.timeout, dispatcher)
            .await
            .with_context(|| self.invocation_context(method))?;
        self.decode(method, terminal)
    }

    pub async fn invoke_with_dispatcher_and_cancel_check<P, R, F>(
        &self,
        method: &str,
        params: &P,
        dispatcher: Arc<dyn AsyncHostRequestDispatcher>,
        is_cancelled: F,
    ) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
        F: Fn() -> bool,
    {
        if is_cancelled() {
            bail!("process module {:?}: {method} was canceled", self.module_id);
        }
        let value = self.encode(method, params)?;
        let mut handle = self
            .broker
            .start_invocation_with_dispatcher(&self.target, method, value, self.timeout, dispatcher)
            .await
            .with_context(|| self.invocation_context(method))?;
        let cancel = handle.cancel_handle();
        let mut terminal = Box::pin(handle.result());
        let mut cancellation_poll = interval(CANCEL_POLL_INTERVAL);
        cancellation_poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let terminal = loop {
            tokio::select! {
                result = &mut terminal => break result,
                _ = cancellation_poll.tick() => {
                    if is_cancelled() {
                        cancel.cancel(CancelCause::User)
                            .with_context(|| self.invocation_context(method))?;
                        break terminal.await;
                    }
                }
            }
        }
        .with_context(|| self.invocation_context(method))?;
        self.decode(method, terminal)
    }

    /// Used while a catalog is being built, before runtime traffic can start.
    pub fn invoke_bootstrap<P, R>(&self, method: &str, params: &P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value = self.encode(method, params)?;
        let terminal = self
            .broker
            .invoke_bootstrap(&self.target, method, value, self.timeout)
            .with_context(|| self.invocation_context(method))?;
        self.decode(method, terminal)
    }

    /// Used only by callback-free synchronous slot traits.
    pub fn invoke_blocking<P, R>(&self, method: &str, params: &P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value = self.encode(method, params)?;
        let terminal = self
            .broker
            .invoke_blocking(&self.target, method, value, self.timeout)
            .with_context(|| self.invocation_context(method))?;
        self.decode(method, terminal)
    }

    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    pub fn component_id(&self) -> &str {
        &self.component_id
    }

    fn encode<P: Serialize>(&self, method: &str, params: &P) -> Result<Value> {
        serde_json::to_value(params).with_context(|| {
            format!(
                "process module {:?}: failed to serialize {method} request",
                self.module_id
            )
        })
    }

    fn invocation_context(&self, method: &str) -> String {
        format!(
            "process component {:?} export {:?}: {method} invocation failed",
            self.component_id, self.module_id
        )
    }

    fn decode<R: DeserializeOwned>(&self, method: &str, terminal: InvocationTerminal) -> Result<R> {
        let value = terminal_value(terminal, &self.module_id, method)?;
        match serde_json::from_value(value) {
            Ok(response) => Ok(response),
            Err(error) => {
                let _ = self.broker.reset();
                Err(error).with_context(|| {
                    format!(
                        "process component {:?} export {:?}: {method} returned an invalid response",
                        self.component_id, self.module_id
                    )
                })
            }
        }
    }
}

fn terminal_value(terminal: InvocationTerminal, module_id: &str, method: &str) -> Result<Value> {
    match terminal {
        InvocationTerminal::Success(value) => Ok(value),
        InvocationTerminal::ModuleError(error) => Err(anyhow::anyhow!(error))
            .with_context(|| format!("process module {module_id:?}: {method} returned an error")),
        InvocationTerminal::Canceled => {
            bail!("process module {module_id:?}: {method} was canceled")
        }
        InvocationTerminal::TimedOut => {
            bail!("process module {module_id:?}: {method} timed out")
        }
        InvocationTerminal::ComponentLost(failure) => {
            bail!("process module {module_id:?}: {method} lost its component: {failure:?}")
        }
    }
}

#[cfg(test)]
mod tests {
    use proteus_module_protocol::{
        ProcessModuleRpcError,
        v3::{ComponentFailure, InvocationTerminal},
    };

    use super::terminal_value;

    #[test]
    fn invocation_terminals_preserve_their_failure_class_in_adapter_errors() {
        let cases = [
            (InvocationTerminal::Canceled, "was canceled"),
            (InvocationTerminal::TimedOut, "timed out"),
            (
                InvocationTerminal::ComponentLost(ComponentFailure::Protocol),
                "lost its component: Protocol",
            ),
            (
                InvocationTerminal::ModuleError(ProcessModuleRpcError::new(-32000, "broken")),
                "returned an error",
            ),
        ];

        for (terminal, expected) in cases {
            let error = terminal_value(terminal, "probe", "run")
                .expect_err("non-success terminal must remain an error");
            assert!(format!("{error:#}").contains(expected), "{error:#}");
        }
    }
}
