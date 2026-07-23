use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    core::{AppConfig, BuiltinRegistry, ModulesConfig},
    domain::{ModelRef, PermissionMode, ReasoningConfig, ToolSpec},
};

pub const CONFIG_SNAPSHOT_FILE: &str = "config_snapshot.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfigSnapshot {
    pub schema_version: u32,
    pub ts: u64,
    pub profile_name: String,
    pub active_provider: String,
    pub model: ModelRef,
    pub reasoning: ReasoningConfig,
    pub modules: SessionConfigModules,
    #[serde(default = "default_subagent_surface")]
    pub subagent_surface: String,
    pub tools: Vec<SessionConfigTool>,
    pub permission_mode_default: PermissionMode,
}

pub type SessionConfigModules = ModulesConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfigTool {
    pub source: String,
    pub spec: ToolSpec,
}

impl SessionConfigSnapshot {
    pub fn from_runtime_config(
        config: &AppConfig,
        registry: &BuiltinRegistry,
        permission_mode_default: PermissionMode,
    ) -> Self {
        let tools = registry
            .tools
            .entries()
            .into_iter()
            .map(|(source, spec)| SessionConfigTool {
                source: source.label(),
                spec,
            })
            .collect();
        Self {
            schema_version: 2,
            ts: unix_timestamp_ms(),
            profile_name: config.profile.name.clone(),
            active_provider: config.active_provider.clone(),
            model: registry.model_config.model_ref(),
            reasoning: registry.model_config.reasoning.clone(),
            modules: config.modules.clone(),
            subagent_surface: config.subagents.surface.as_str().to_owned(),
            tools,
            permission_mode_default,
        }
    }
}

fn default_subagent_surface() -> String {
    "task".to_owned()
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

pub fn write_config_snapshot(session_dir: &Path, snapshot: &SessionConfigSnapshot) -> Result<()> {
    std::fs::create_dir_all(session_dir)
        .with_context(|| format!("failed to create session dir {}", session_dir.display()))?;
    let path = session_dir.join(CONFIG_SNAPSHOT_FILE);
    let mut content = serde_json::to_vec_pretty(snapshot)?;
    content.push(b'\n');
    std::fs::write(&path, content)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}
