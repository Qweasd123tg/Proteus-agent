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
        PROCESS_SEARCH_CONTRACT_VERSION, PROCESS_SEARCH_METHOD, ProcessSearchResponse,
        SearchBackend, SearchQuery,
    },
    core::expand_user_path,
    domain::ContextChunk,
};

const DEFAULT_PROCESS_SEARCH_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSearchConfig {
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
    /// Module-owned payload sent only through the v1 initialize contract.
    #[serde(default = "default_module_config")]
    config: Value,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_PROCESS_SEARCH_TIMEOUT_MS
}

fn default_module_config() -> Value {
    Value::Object(serde_json::Map::new())
}

impl ProcessSearchConfig {
    pub fn from_value(config: Value) -> Result<Self> {
        if config.is_null() {
            bail!("module_config.search.process is required when modules.search = \"process\"");
        }
        let config: Self = serde_json::from_value(config)
            .context("failed to parse module_config.search.process")?;
        validate_config(&config)?;
        Ok(config)
    }

    /// Builds the launch description without starting the child. Read-only
    /// diagnostics use this to validate config and command resolution without
    /// triggering the module handshake.
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

/// `SearchBackend` implemented by one persistent newline-JSON child process.
pub struct ProcessSearchBackend {
    module_id: String,
    timeout: Duration,
    session: Arc<ProcessModuleSession>,
}

impl ProcessSearchBackend {
    pub fn from_config(config: Value, workspace: &Path) -> Result<Self> {
        let config = ProcessSearchConfig::from_value(config)?;
        let spec = config.process_spec(workspace)?;
        let timeout = config.timeout();
        let expected_module_id = config.module_id.clone();
        let binding = ProcessModuleBinding::new(
            "search",
            expected_module_id.clone(),
            PROCESS_SEARCH_CONTRACT_VERSION,
            config.config,
        )?;
        let session = Arc::new(ProcessModuleSession::connect(
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
            session,
        })
    }
}

#[async_trait]
impl SearchBackend for ProcessSearchBackend {
    async fn search(&self, query: SearchQuery) -> Result<Vec<ContextChunk>> {
        let session = Arc::clone(&self.session);
        let module_id = self.module_id.clone();
        let timeout = self.timeout;
        tokio::task::spawn_blocking(move || {
            let params = serde_json::to_value(query)
                .context("process search: failed to serialize SearchQuery")?;
            let invocation = session
                .invoke(PROCESS_SEARCH_METHOD, params, timeout)
                .with_context(|| format!("process search module {module_id:?} request failed"))?;
            let value = terminal_value(invocation.terminal, &module_id)?;
            match serde_json::from_value::<ProcessSearchResponse>(value) {
                Ok(response) => Ok(response.chunks),
                Err(error) => {
                    // A syntactically valid JSON-RPC response with the wrong
                    // slot payload poisons this session. Restart lazily on the
                    // next call instead of continuing an unknown protocol.
                    session.reset();
                    Err(error).with_context(|| {
                        format!("process search module {module_id:?} returned invalid response")
                    })
                }
            }
        })
        .await
        .map_err(|error| anyhow::anyhow!("process search join error: {error}"))?
    }
}

fn terminal_value(terminal: ProcessModuleTerminal, module_id: &str) -> Result<Value> {
    match terminal {
        ProcessModuleTerminal::Success(value) => Ok(value),
        ProcessModuleTerminal::ModuleError(error) => Err(anyhow::anyhow!(error))
            .with_context(|| format!("process search module {module_id:?} returned an error")),
        ProcessModuleTerminal::Canceled => {
            bail!("process search module {module_id:?} invocation was canceled")
        }
        ProcessModuleTerminal::TimedOut => {
            bail!("process search module {module_id:?} invocation timed out")
        }
    }
}

fn validate_config(config: &ProcessSearchConfig) -> Result<()> {
    if config.module_id.trim().is_empty() {
        bail!("module_config.search.process.module_id must not be empty");
    }
    if config.command.trim().is_empty() {
        bail!("module_config.search.process.command must not be empty");
    }
    if config.timeout_ms == 0 {
        bail!("module_config.search.process.timeout_ms must be greater than zero");
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
        .with_context(|| format!("failed to resolve process search cwd {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_module_config_becomes_an_empty_object() {
        let config = ProcessSearchConfig::from_value(serde_json::json!({
            "module_id": "fixture",
            "command": "python3"
        }))
        .expect("minimal process search config");

        assert_eq!(config.config, serde_json::json!({}));
    }

    #[test]
    fn config_rejects_unknown_fields_and_zero_timeout() {
        let unknown = serde_json::json!({
            "module_id": "fixture",
            "command": "python3",
            "legacy_command": "python",
        });
        serde_json::from_value::<ProcessSearchConfig>(unknown)
            .expect_err("legacy fields must be rejected");

        let zero_timeout = ProcessSearchConfig {
            module_id: "fixture".to_owned(),
            command: "python3".to_owned(),
            args: Vec::new(),
            cwd: None,
            env_allowlist: Vec::new(),
            env: BTreeMap::new(),
            config: Value::Null,
            timeout_ms: 0,
        };
        assert!(validate_config(&zero_timeout).is_err());
    }
}
