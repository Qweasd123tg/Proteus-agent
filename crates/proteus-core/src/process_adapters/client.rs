use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use proteus_module_protocol::{
    HostRequestDispatcher, ProcessModuleBinding, ProcessModuleSession, ProcessModuleSessionOptions,
    ProcessModuleTerminal,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::ProcessAdapterConfig;

/// Shared strict client used by typed process slot adapters.
pub struct ProcessModuleClient {
    module_id: String,
    timeout: Duration,
    session: Arc<ProcessModuleSession>,
}

impl ProcessModuleClient {
    pub fn connect(
        slot: &str,
        contract_version: &str,
        config: ProcessAdapterConfig,
        workspace: &Path,
        default_timeout_ms: u64,
    ) -> Result<Self> {
        config.validate_for("process_modules[]", default_timeout_ms)?;
        if config.slot() != slot {
            bail!(
                "process module {:?} declares slot {:?}, expected {:?}",
                config.module_id(),
                config.slot(),
                slot
            );
        }
        let spec = config.process_spec(workspace)?;
        let timeout = config.timeout(default_timeout_ms);
        let handshake_timeout = config.handshake_timeout(default_timeout_ms);
        let module_id = config.module_id().to_owned();
        let binding = ProcessModuleBinding::new(
            slot,
            module_id.clone(),
            contract_version,
            config.module_config(),
        )?;
        let session = Arc::new(ProcessModuleSession::connect(
            spec,
            binding,
            ProcessModuleSessionOptions {
                handshake_timeout,
                ..ProcessModuleSessionOptions::default()
            },
        )?);
        Ok(Self {
            module_id,
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
            .invoke(method, value, self.timeout)
            .with_context(|| {
                format!(
                    "process module {:?}: {method} invocation failed",
                    self.module_id
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
                method,
                value,
                self.timeout,
                dispatcher,
                is_cancelled,
            )
            .with_context(|| {
                format!(
                    "process module {:?}: {method} invocation failed",
                    self.module_id
                )
            })?;
        self.decode(method, invocation.terminal)
    }

    pub fn module_id(&self) -> &str {
        &self.module_id
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
                        "process module {:?}: {method} returned an invalid response",
                        self.module_id
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
