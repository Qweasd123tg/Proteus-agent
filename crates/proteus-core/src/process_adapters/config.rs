use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use proteus_process_host::ProcessSpec;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::expand_user_path;

/// Launch shape shared by every process-module slot. Slot adapters own
/// methods and authority; this type only carries process identity and opaque
/// module config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessAdapterConfig {
    slot: String,
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
    #[serde(skip, default = "empty_object")]
    config: Value,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    handshake_timeout_ms: Option<u64>,
    #[serde(default)]
    description: Option<String>,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

impl ProcessAdapterConfig {
    pub fn from_value(value: Value, path: &str, default_timeout_ms: u64) -> Result<Self> {
        if value.is_null() {
            bail!("{path} is required");
        }
        let config: Self =
            serde_json::from_value(value).with_context(|| format!("failed to parse {path}"))?;
        config.validate(path, default_timeout_ms)?;
        Ok(config)
    }

    pub fn process_spec(&self, workspace: &Path) -> Result<ProcessSpec> {
        Ok(ProcessSpec::new(self.command.clone())
            .args(self.args.clone())
            .env_allowlist(self.env_allowlist.clone())
            .envs(self.env.clone())
            .cwd(resolve_process_cwd(self.cwd.as_deref(), workspace)?))
    }

    pub fn validate_for(&self, path: &str, default_timeout_ms: u64) -> Result<()> {
        self.validate(path, default_timeout_ms)
    }

    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    pub fn slot(&self) -> &str {
        &self.slot
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn module_config(&self) -> Value {
        self.config.clone()
    }

    pub(crate) fn with_module_config(mut self, config: Value) -> Result<Self> {
        if !config.is_object() {
            bail!(
                "module_config.{}.{} must be an object",
                self.slot,
                self.module_id
            );
        }
        self.config = config;
        Ok(self)
    }

    pub fn timeout(&self, default_timeout_ms: u64) -> Duration {
        Duration::from_millis(self.timeout_ms.unwrap_or(default_timeout_ms))
    }

    pub fn handshake_timeout(&self, default_timeout_ms: u64) -> Duration {
        Duration::from_millis(
            self.handshake_timeout_ms
                .or(self.timeout_ms)
                .unwrap_or(default_timeout_ms),
        )
    }

    fn validate(&self, path: &str, default_timeout_ms: u64) -> Result<()> {
        if self.slot.trim().is_empty() {
            bail!("{path}.slot must not be empty");
        }
        if self.module_id.trim().is_empty() {
            bail!("{path}.module_id must not be empty");
        }
        if self.command.trim().is_empty() {
            bail!("{path}.command must not be empty");
        }
        if self.timeout_ms == Some(0) || self.handshake_timeout_ms == Some(0) {
            bail!("{path} timeouts must be greater than zero");
        }
        if default_timeout_ms == 0 && self.timeout_ms.is_none() {
            bail!("{path}.timeout_ms is required when the slot has no finite default");
        }
        if !self.config.is_object() {
            bail!("{path}.config must be an object");
        }
        Ok(())
    }
}

fn resolve_process_cwd(configured: Option<&Path>, workspace: &Path) -> Result<PathBuf> {
    let path = configured.map_or_else(|| workspace.to_path_buf(), expand_user_path);
    let path = if path.is_relative() {
        workspace.join(path)
    } else {
        path
    };
    std::fs::canonicalize(&path)
        .with_context(|| format!("failed to resolve process module cwd {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_is_strict_and_payload_is_an_object() {
        let config = ProcessAdapterConfig::from_value(
            serde_json::json!({
                "slot": "search",
                "module_id": "fixture",
                "command": "worker",
                "description": "fixture"
            }),
            "modules.fixture",
            1_000,
        )
        .expect("valid config");
        assert_eq!(config.module_id(), "fixture");
        assert_eq!(config.module_config(), serde_json::json!({}));

        ProcessAdapterConfig::from_value(
            serde_json::json!({
                "slot": "search",
                "module_id": "fixture",
                "command": "worker",
                "legacy": true
            }),
            "modules.fixture",
            1_000,
        )
        .expect_err("unknown launch fields must fail");
        ProcessAdapterConfig::from_value(
            serde_json::json!({
                "slot": "search",
                "module_id": "fixture",
                "command": "worker",
                "timeout_ms": 0
            }),
            "modules.fixture",
            1_000,
        )
        .expect_err("zero timeout must fail");
    }
}
