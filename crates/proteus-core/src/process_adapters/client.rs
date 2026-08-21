use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use proteus_module_protocol::{
    HostRequestDispatcher, ProcessComponentSession, ProcessModuleTerminal,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::ProcessExportConfig;

/// Typed view of one export on a shared strict-v2 process component session.
pub struct ProcessExportClient {
    component_id: String,
    module_id: String,
    target: proteus_contracts::contracts::ProcessComponentExportRef,
    timeout: Duration,
    session: Arc<ProcessComponentSession>,
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
        let session = config.connect(workspace)?;
        Ok(Self {
            component_id,
            module_id,
            target,
            timeout,
            session,
        })
    }

    pub fn invoke<P, R>(&self, method: &str, params: &P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value = serde_json::to_value(params).with_context(|| {
            format!(
                "process module {:?}: failed to serialize {method} request",
                self.module_id
            )
        })?;
        let invocation = self
            .session
            .invoke(&self.target, method, value, self.timeout)
            .with_context(|| {
                format!(
                    "process component {:?} export {:?}: {method} invocation failed",
                    self.component_id, self.module_id
                )
            })?;
        self.decode(method, invocation.terminal)
    }

    pub fn invoke_with_dispatcher<P, R>(
        &self,
        method: &str,
        params: &P,
        dispatcher: Arc<dyn HostRequestDispatcher>,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value = serde_json::to_value(params).with_context(|| {
            format!(
                "process module {:?}: failed to serialize {method} request",
                self.module_id
            )
        })?;
        let invocation = self
            .session
            .invoke_with_dispatcher_and_cancel_check(
                &self.target,
                method,
                value,
                self.timeout,
                dispatcher,
                is_cancelled,
            )
            .with_context(|| {
                format!(
                    "process component {:?} export {:?}: {method} invocation failed",
                    self.component_id, self.module_id
                )
            })?;
        self.decode(method, invocation.terminal)
    }

    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    pub fn component_id(&self) -> &str {
        &self.component_id
    }

    fn decode<R: DeserializeOwned>(
        &self,
        method: &str,
        terminal: ProcessModuleTerminal,
    ) -> Result<R> {
        let value = terminal_value(terminal, &self.module_id, method)?;
        match serde_json::from_value(value) {
            Ok(response) => Ok(response),
            Err(error) => {
                self.session.reset();
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

fn terminal_value(terminal: ProcessModuleTerminal, module_id: &str, method: &str) -> Result<Value> {
    match terminal {
        ProcessModuleTerminal::Success(value) => Ok(value),
        ProcessModuleTerminal::ModuleError(error) => Err(anyhow::anyhow!(error))
            .with_context(|| format!("process module {module_id:?}: {method} returned an error")),
        ProcessModuleTerminal::Canceled => {
            bail!("process module {module_id:?}: {method} was canceled")
        }
        ProcessModuleTerminal::TimedOut => {
            bail!("process module {module_id:?}: {method} timed out")
        }
    }
}
