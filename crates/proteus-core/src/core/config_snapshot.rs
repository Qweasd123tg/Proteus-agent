use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    core::{AppConfig, ModulesConfig, RuntimeRegistry},
    domain::{ModelRef, PermissionMode, ReasoningConfig, ToolSpec},
};

pub const CONFIG_SNAPSHOT_FILE: &str = "config_snapshot.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConfigSnapshot {
    pub schema_version: u32,
    pub ts: u64,
    pub profile_name: String,
    pub active_provider: String,
    pub model: ModelRef,
    pub reasoning: ReasoningConfig,
    pub modules: SessionConfigModules,
    pub agent_control_surface: String,
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
        registry: &RuntimeRegistry,
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
            schema_version: 3,
            ts: unix_timestamp_ms(),
            profile_name: config.profile.name.clone(),
            active_provider: config.active_provider.clone(),
            model: registry.model_config.model_ref(),
            reasoning: registry.model_config.reasoning.clone(),
            modules: config.modules.clone(),
            agent_control_surface: config.agent_control.surface.as_str().to_owned(),
            tools,
            permission_mode_default,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_subagent_surface_is_rejected_without_alias() {
        let snapshot = SessionConfigSnapshot {
            schema_version: 3,
            ts: 0,
            profile_name: "test".to_owned(),
            active_provider: "fake".to_owned(),
            model: ModelRef::new("fake", "fake"),
            reasoning: ReasoningConfig::default(),
            modules: ModulesConfig::default(),
            agent_control_surface: "task".to_owned(),
            tools: Vec::new(),
            permission_mode_default: PermissionMode::Normal,
        };
        let mut value = serde_json::to_value(snapshot).expect("snapshot value");
        let object = value.as_object_mut().expect("snapshot object");
        let surface = object
            .remove("agent_control_surface")
            .expect("current surface");
        object.insert("subagent_surface".to_owned(), surface);

        let error = serde_json::from_value::<SessionConfigSnapshot>(value)
            .expect_err("legacy snapshot field must fail closed");
        assert!(
            error
                .to_string()
                .contains("unknown field `subagent_surface`")
        );
    }
}
