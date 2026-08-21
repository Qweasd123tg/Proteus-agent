use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use proteus_module_protocol::{
    ProcessComponentBinding, ProcessComponentSession, ProcessComponentSessionOptions,
    ProcessExportBinding,
};
use proteus_process_host::ProcessSpec;
use serde::{Deserialize, Serialize};

use crate::core::expand_user_path;

const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 30_000;

/// Host-owned launch description for one persistent process component.
///
/// Export identities are nested as `exports.<slot>.<module_id>` so config
/// includes merge them as objects instead of replacing one descriptor array.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessComponentConfig {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    env_allowlist: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    handshake_timeout_ms: Option<u64>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    exports: BTreeMap<String, BTreeMap<String, ProcessExportLaunchConfig>>,
}

/// Per-export launch controls. Contract identity comes from the containing map
/// keys; opaque implementation config remains in `module_config.<slot>.<id>`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessExportLaunchConfig {
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    description: Option<String>,
}

impl ProcessComponentConfig {
    pub fn process_spec(&self, workspace: &Path) -> Result<ProcessSpec> {
        Ok(ProcessSpec::new(self.command.clone())
            .args(self.args.clone())
            .env_allowlist(self.env_allowlist.clone())
            .envs(self.env.clone())
            .cwd(resolve_process_cwd(self.cwd.as_deref(), workspace)?))
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn exports(&self) -> impl Iterator<Item = (&str, &str, &ProcessExportLaunchConfig)> {
        self.exports.iter().flat_map(|(slot, modules)| {
            modules
                .iter()
                .map(move |(module_id, launch)| (slot.as_str(), module_id.as_str(), launch))
        })
    }

    pub fn validate_for(&self, component_id: &str, path: &str) -> Result<()> {
        if component_id.trim().is_empty() {
            bail!("{path} component id must not be empty");
        }
        if self.command.trim().is_empty() {
            bail!("{path}.command must not be empty");
        }
        if self.handshake_timeout_ms == Some(0) {
            bail!("{path}.handshake_timeout_ms must be greater than zero");
        }
        if self.exports.is_empty() {
            bail!("{path}.exports must not be empty");
        }
        for (slot, modules) in &self.exports {
            if slot.trim().is_empty() {
                bail!("{path}.exports contains an empty slot");
            }
            if modules.is_empty() {
                bail!("{path}.exports.{slot} must not be empty");
            }
            for (module_id, export) in modules {
                if module_id.trim().is_empty() {
                    bail!("{path}.exports.{slot} contains an empty module id");
                }
                if export.timeout_ms == Some(0) {
                    bail!("{path}.exports.{slot}.{module_id}.timeout_ms must be greater than zero");
                }
            }
        }
        Ok(())
    }

    fn handshake_timeout(&self) -> Duration {
        Duration::from_millis(
            self.handshake_timeout_ms
                .unwrap_or(DEFAULT_HANDSHAKE_TIMEOUT_MS),
        )
    }

    fn export(&self, slot: &str, module_id: &str) -> Option<&ProcessExportLaunchConfig> {
        self.exports.get(slot)?.get(module_id)
    }
}

/// Runtime launch handle shared by every typed adapter exported from one
/// component. A catalog owns one launcher; adapters built for the same
/// canonical workspace therefore share one persistent session and lifecycle.
pub(crate) struct ProcessComponentLauncher {
    component_id: String,
    config: ProcessComponentConfig,
    binding: ProcessComponentBinding,
    sessions: Mutex<HashMap<PathBuf, Arc<ProcessComponentSession>>>,
}

impl std::fmt::Debug for ProcessComponentLauncher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProcessComponentLauncher")
            .field("component_id", &self.component_id)
            .field("config", &self.config)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl ProcessComponentLauncher {
    pub(crate) fn new(
        component_id: impl Into<String>,
        config: ProcessComponentConfig,
        exports: Vec<ProcessExportBinding>,
    ) -> Result<Arc<Self>> {
        let component_id = component_id.into();
        let binding = ProcessComponentBinding::new(component_id.clone(), exports)?;
        Ok(Arc::new(Self {
            component_id,
            config,
            binding,
            sessions: Mutex::new(HashMap::new()),
        }))
    }

    pub(crate) fn export(
        self: &Arc<Self>,
        slot: &str,
        module_id: &str,
    ) -> Result<ProcessExportConfig> {
        let launch = self.config.export(slot, module_id).ok_or_else(|| {
            anyhow::anyhow!(
                "component {:?} has no configured export {slot}/{module_id}",
                self.component_id
            )
        })?;
        let binding = self
            .binding
            .exports
            .iter()
            .find(|binding| binding.slot == slot && binding.module_id == module_id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "component {:?} has no bound export {slot}/{module_id}",
                    self.component_id
                )
            })?;
        Ok(ProcessExportConfig {
            launcher: Arc::clone(self),
            binding,
            timeout_ms: launch.timeout_ms,
            description: launch.description.clone(),
        })
    }

    fn connect(&self, workspace: &Path) -> Result<Arc<ProcessComponentSession>> {
        let workspace = std::fs::canonicalize(workspace).with_context(|| {
            format!(
                "failed to resolve component {:?} workspace {}",
                self.component_id,
                workspace.display()
            )
        })?;
        let mut sessions = self.sessions.lock().map_err(|_| {
            anyhow::anyhow!("component {:?} session cache poisoned", self.component_id)
        })?;
        if let Some(session) = sessions.get(&workspace) {
            return Ok(Arc::clone(session));
        }
        let session = Arc::new(ProcessComponentSession::connect(
            self.config.process_spec(&workspace)?,
            self.binding.clone(),
            ProcessComponentSessionOptions {
                handshake_timeout: self.config.handshake_timeout(),
                ..ProcessComponentSessionOptions::default()
            },
        )?);
        sessions.insert(workspace, Arc::clone(&session));
        Ok(session)
    }
}

/// One typed module export plus a shared component launcher.
#[derive(Debug, Clone)]
pub struct ProcessExportConfig {
    launcher: Arc<ProcessComponentLauncher>,
    binding: ProcessExportBinding,
    timeout_ms: Option<u64>,
    description: Option<String>,
}

impl ProcessExportConfig {
    pub fn component_id(&self) -> &str {
        &self.launcher.component_id
    }

    pub fn module_id(&self) -> &str {
        &self.binding.module_id
    }

    pub fn slot(&self) -> &str {
        &self.binding.slot
    }

    pub fn description(&self) -> Option<&str> {
        self.description
            .as_deref()
            .or_else(|| self.launcher.config.description())
    }

    pub fn timeout(&self, default_timeout_ms: u64) -> Duration {
        Duration::from_millis(self.timeout_ms.unwrap_or(default_timeout_ms))
    }

    pub fn validate_for(&self, slot: &str, default_timeout_ms: u64) -> Result<()> {
        if self.slot() != slot {
            bail!(
                "component {:?} export {:?} declares slot {:?}, expected {:?}",
                self.component_id(),
                self.module_id(),
                self.slot(),
                slot
            );
        }
        if default_timeout_ms == 0 && self.timeout_ms.is_none() {
            bail!(
                "component {:?} export {}/{} requires timeout_ms",
                self.component_id(),
                self.slot(),
                self.module_id()
            );
        }
        Ok(())
    }

    pub(crate) fn binding(&self) -> &ProcessExportBinding {
        &self.binding
    }

    pub(crate) fn connect(&self, workspace: &Path) -> Result<Arc<ProcessComponentSession>> {
        self.launcher.connect(workspace)
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
        .with_context(|| format!("failed to resolve process component cwd {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_config_is_strict_and_exports_are_nested_by_identity() {
        let config: ProcessComponentConfig = serde_json::from_value(serde_json::json!({
            "command": "worker",
            "description": "fixture",
            "exports": {
                "search": {
                    "fixture": {"timeout_ms": 1000}
                }
            }
        }))
        .expect("valid config");
        config
            .validate_for("fixture-component", "components.fixture-component")
            .expect("valid component");
        assert_eq!(
            config
                .exports()
                .map(|(slot, id, _)| (slot, id))
                .collect::<Vec<_>>(),
            [("search", "fixture")]
        );

        serde_json::from_value::<ProcessComponentConfig>(serde_json::json!({
            "command": "worker",
            "legacy_slot": "search",
            "exports": {"search": {"fixture": {}}}
        }))
        .expect_err("unknown launch fields must fail");
    }

    #[test]
    fn component_config_rejects_empty_exports_and_zero_timeouts() {
        let empty: ProcessComponentConfig = serde_json::from_value(serde_json::json!({
            "command": "worker",
            "exports": {}
        }))
        .expect("shape");
        empty
            .validate_for("fixture", "components.fixture")
            .expect_err("empty exports must fail");

        let zero: ProcessComponentConfig = serde_json::from_value(serde_json::json!({
            "command": "worker",
            "exports": {"search": {"fixture": {"timeout_ms": 0}}}
        }))
        .expect("shape");
        zero.validate_for("fixture", "components.fixture")
            .expect_err("zero export timeout must fail");
    }
}
