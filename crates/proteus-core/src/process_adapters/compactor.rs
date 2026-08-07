use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use proteus_module_protocol::{
    ProcessModuleBinding, ProcessModuleSession, ProcessModuleSessionOptions, ProcessModuleTerminal,
};
use proteus_process_host::ProcessSpec;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    contracts::{
        CompactionHost, CompactionInput, CompactionOutput, HistoryCompactor,
        PROCESS_COMPACTOR_CONTRACT_VERSION, PROCESS_COMPACTOR_METHOD, ProcessCompactionResponse,
    },
    core::expand_user_path,
};

const DEFAULT_PROCESS_COMPACTOR_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessCompactorConfig {
    module_id: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    env_allowlist: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    /// Module-owned payload copied to `CompactionInput.config`. Launch and
    /// environment details stay on the core side of the process boundary.
    #[serde(default)]
    strategy: Value,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_PROCESS_COMPACTOR_TIMEOUT_MS
}

impl ProcessCompactorConfig {
    pub fn from_value(config: Value) -> Result<Self> {
        if config.is_null() {
            bail!(
                "module_config.compactor.process is required when modules.compactor = \"process\""
            );
        }
        let config: Self = serde_json::from_value(config)
            .context("failed to parse module_config.compactor.process")?;
        validate_config(&config)?;
        Ok(config)
    }

    /// Builds the launch description without starting the child. Read-only
    /// diagnostics use this to validate config and environment resolution.
    pub fn process_spec(&self, workspace: &Path) -> Result<ProcessSpec> {
        Ok(ProcessSpec::new(self.command.clone())
            .args(self.args.clone())
            .env_allowlist(self.env_allowlist.clone())
            .envs(self.env.clone())
            .cwd(resolve_process_cwd(self.cwd.as_deref(), workspace)?))
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

/// `HistoryCompactor` implemented by one persistent newline-JSON child.
///
/// The process is a pure `CompactionInput -> CompactionOutput` module. It does
/// not receive `CompactionHost` capabilities and therefore cannot make hidden
/// model calls or bypass the configured model adapter.
pub struct ProcessHistoryCompactor {
    module_id: String,
    timeout: Duration,
    strategy: Value,
    process: Arc<ProcessModuleSession>,
}

impl ProcessHistoryCompactor {
    pub fn from_config(config: Value, workspace: &Path) -> Result<Self> {
        let config = ProcessCompactorConfig::from_value(config)?;
        let spec = config.process_spec(workspace)?;
        let timeout = config.timeout();
        let expected_module_id = config.module_id.clone();
        let binding = ProcessModuleBinding::new(
            "compactor",
            expected_module_id.clone(),
            PROCESS_COMPACTOR_CONTRACT_VERSION,
            config.strategy.clone(),
        )?;
        let process = Arc::new(ProcessModuleSession::connect(
            spec,
            binding,
            ProcessModuleSessionOptions {
                handshake_timeout: timeout,
                ..ProcessModuleSessionOptions::default()
            },
        )?);

        Ok(Self {
            module_id: expected_module_id,
            timeout,
            strategy: config.strategy,
            process,
        })
    }
}

#[async_trait]
impl HistoryCompactor for ProcessHistoryCompactor {
    async fn compact(
        &self,
        input: CompactionInput,
        host: Arc<dyn CompactionHost>,
    ) -> Result<CompactionOutput> {
        if host.is_cancelled() {
            bail!("turn canceled by client");
        }

        let input = input.with_config(self.strategy.clone());
        let process = Arc::clone(&self.process);
        let cancellation_host = Arc::clone(&host);
        let module_id = self.module_id.clone();
        let timeout = self.timeout;
        let response = tokio::task::spawn_blocking(move || {
            let params = serde_json::to_value(input)
                .context("process compactor: failed to serialize CompactionInput")?;
            let invocation = process
                .invoke_with_cancel_check(PROCESS_COMPACTOR_METHOD, params, timeout, || {
                    cancellation_host.is_cancelled()
                })
                .with_context(|| {
                    format!("process compactor module {module_id:?} request failed")
                })?;
            let value = terminal_value(invocation.terminal, &module_id)?;
            match serde_json::from_value::<ProcessCompactionResponse>(value) {
                Ok(response) => Ok(response),
                Err(error) => {
                    process.reset();
                    Err(error).with_context(|| {
                        format!("process compactor module {module_id:?} returned invalid response")
                    })
                }
            }
        })
        .await
        .map_err(|error| anyhow::anyhow!("process compactor join error: {error}"))??;

        if host.is_cancelled() {
            bail!("turn canceled by client");
        }
        Ok(response.output)
    }
}

fn terminal_value(terminal: ProcessModuleTerminal, module_id: &str) -> Result<Value> {
    match terminal {
        ProcessModuleTerminal::Success(value) => Ok(value),
        ProcessModuleTerminal::ModuleError(error) => Err(anyhow::anyhow!(error))
            .with_context(|| format!("process compactor module {module_id:?} returned an error")),
        ProcessModuleTerminal::Canceled => {
            bail!("process compactor module {module_id:?} invocation was canceled")
        }
        ProcessModuleTerminal::TimedOut => {
            bail!("process compactor module {module_id:?} invocation timed out")
        }
    }
}

fn validate_config(config: &ProcessCompactorConfig) -> Result<()> {
    if config.module_id.trim().is_empty() {
        bail!("module_config.compactor.process.module_id must not be empty");
    }
    if config.command.trim().is_empty() {
        bail!("module_config.compactor.process.command must not be empty");
    }
    if config.timeout_ms == 0 {
        bail!("module_config.compactor.process.timeout_ms must be greater than zero");
    }
    Ok(())
}

fn resolve_process_cwd(configured: Option<&Path>, workspace: &Path) -> Result<PathBuf> {
    let path = configured.map_or_else(|| workspace.to_path_buf(), expand_user_path);
    let path = if path.is_relative() {
        workspace.join(path)
    } else {
        path
    };
    std::fs::canonicalize(&path)
        .with_context(|| format!("failed to resolve process compactor cwd {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_unknown_fields_and_zero_timeout() {
        let unknown = serde_json::json!({
            "module_id": "fixture",
            "command": "python3",
            "legacy_command": "python",
        });
        serde_json::from_value::<ProcessCompactorConfig>(unknown)
            .expect_err("legacy fields must be rejected");

        let zero_timeout = ProcessCompactorConfig {
            module_id: "fixture".to_owned(),
            command: "python3".to_owned(),
            args: Vec::new(),
            cwd: None,
            env_allowlist: Vec::new(),
            env: BTreeMap::new(),
            timeout_ms: 0,
            strategy: Value::Null,
        };
        assert!(validate_config(&zero_timeout).is_err());
    }
}
