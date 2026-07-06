use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    core::{AppConfig, BuiltinRegistry, unix_timestamp_ms},
    domain::{ModelRef, PermissionMode, ReasoningConfig, ToolSpec},
};

pub const CONFIG_SNAPSHOT_FILE: &str = "config_snapshot.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfigSnapshot {
    pub schema_version: u32,
    pub ts: u64,
    pub profile_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider: Option<String>,
    pub model: ModelRef,
    pub reasoning: ReasoningConfig,
    pub modules: SessionConfigModules,
    pub tools: Vec<SessionConfigTool>,
    pub permission_mode_default: PermissionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfigModules {
    pub workflow: String,
    pub search: String,
    pub memory: String,
    pub memory_policy: String,
    pub context: String,
    pub policy: String,
    pub patch: String,
    pub compactor: String,
    pub tool_exposure: String,
    pub subagent: String,
    pub renderer: String,
}

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
            schema_version: 1,
            ts: unix_timestamp_ms(),
            profile_name: config.profile.name.clone(),
            active_provider: config.active_provider.clone(),
            model: registry.model_config.model_ref(),
            reasoning: registry.model_config.reasoning.clone(),
            modules: SessionConfigModules {
                workflow: config.modules.workflow.clone(),
                search: config.modules.search.clone(),
                memory: config.modules.memory.clone(),
                memory_policy: config.modules.memory_policy.clone(),
                context: config.modules.context.clone(),
                policy: config.modules.policy.clone(),
                patch: config.modules.patch.clone(),
                compactor: config.modules.compactor.clone(),
                tool_exposure: config.modules.tool_exposure.clone(),
                subagent: config.modules.subagent.clone(),
                renderer: config.modules.renderer.clone(),
            },
            tools,
            permission_mode_default,
        }
    }
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
