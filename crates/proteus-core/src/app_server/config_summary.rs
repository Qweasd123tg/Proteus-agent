use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{contracts::ToolSource, core::AppConfig, domain::PermissionMode};

pub(super) fn render_config_summary(
    config: &AppConfig,
    config_path: Option<&Path>,
    cwd: &Path,
    mode: PermissionMode,
    tools: &[(ToolSource, crate::domain::ToolSpec)],
    module_epoch: crate::core::ModuleEpoch,
) -> String {
    let mut lines = Vec::new();
    lines.push("Config summary".to_owned());
    lines.push(format!(
        "config path: {}",
        config_path
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "(default discovery / none)".to_owned())
    ));
    let config_files = config_files(config_path);
    if !config_files.is_empty() {
        lines.push("config files:".to_owned());
        for path in config_files {
            lines.push(format!("  - {}", path.display()));
        }
    }
    lines.push(format!("cwd: {}", cwd.display()));
    lines.push(format!("profile: {}", config.profile.name));
    lines.push(format!("module epoch: {}", module_epoch.as_u64()));
    if let Ok(model) = config.active_model_config() {
        lines.push(format!("model: {}/{}", model.provider, model.model));
    }
    lines.push(format!("permission mode: {mode:?}"));
    lines.push("modules:".to_owned());
    for (kind, id) in config.modules.iter() {
        lines.push(format!("  {}: {id}", kind.as_str()));
    }
    lines.push(format!(
        "subagent surface: {}",
        config.agent_control.surface.as_str()
    ));

    lines.push("tools.enabled:".to_owned());
    if config.tools.enabled.is_empty() {
        lines.push("  (none)".to_owned());
    } else {
        for tool in &config.tools.enabled {
            lines.push(format!("  - {tool}"));
        }
    }

    lines.push("registered tools:".to_owned());
    if tools.is_empty() {
        lines.push("  (none)".to_owned());
    } else {
        for (source, spec) in tools {
            lines.push(format!(
                "  - {} [{} {:?}] {}",
                spec.name,
                source.label(),
                spec.safety,
                spec.description
            ));
        }
    }

    lines.join("\n")
}

pub(super) fn configured_model_options(config: &AppConfig) -> Vec<crate::domain::ModelRef> {
    let mut options = Vec::new();
    if let Ok(model) = config.active_model_config() {
        options.push(model.model_ref());
    }
    for profile in config.providers.values() {
        if let Ok(model) = profile.to_model_config() {
            let model_ref = model.model_ref();
            if !options.iter().any(|item| item == &model_ref) {
                options.push(model_ref);
            }
        }
    }
    options
}

pub(super) fn configured_reasoning_effort_options(
    config: &AppConfig,
    active_model: &crate::domain::ModelRef,
    reasoning: &crate::domain::ReasoningConfig,
) -> Vec<String> {
    let mut options = Vec::new();
    for profile in matching_provider_profiles(config, active_model) {
        push_unique_strings(&mut options, &profile.reasoning_efforts);
        if let Some(effort) = profile.reasoning.effort.as_deref() {
            push_unique(&mut options, effort);
        }
    }

    if let Some(effort) = reasoning.effort.as_deref() {
        push_unique(&mut options, effort);
    }

    options
}

fn matching_provider_profiles<'a>(
    config: &'a AppConfig,
    active_model: &crate::domain::ModelRef,
) -> Vec<&'a crate::core::ProviderProfileConfig> {
    let mut profiles = Vec::new();
    if let Some(profile) = active_provider_profile(config) {
        profiles.push(profile);
    }
    profiles.extend(config.providers.values().filter(|profile| {
        profile.provider == active_model.provider && profile.model == active_model.model
    }));
    profiles
}

fn active_provider_profile(config: &AppConfig) -> Option<&crate::core::ProviderProfileConfig> {
    config.providers.get(&config.active_provider)
}

fn push_unique_strings(options: &mut Vec<String>, values: &[String]) {
    for value in values {
        push_unique(options, value);
    }
}

fn push_unique(options: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() || options.iter().any(|item| item == value) {
        return;
    }
    options.push(value.to_owned());
}

pub(super) fn config_files(config_path: Option<&Path>) -> Vec<PathBuf> {
    let Some(path) = config_path else {
        return Vec::new();
    };
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| matches!(extension, "toml" | "json"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

impl super::AppServerHandle {
    pub async fn config_summary(&self) -> Value {
        use proteus_contracts::app_protocol::config::*;
        let mode = self.permission_mode().await;
        let model_ref = self.runtime.model_ref().await;
        let reasoning = self.runtime.reasoning().await;
        let module_epoch = self.runtime.module_epoch().await;
        let config = self.config.read().await.clone();
        let selection = super::model_selection::selection_summary(
            &config,
            &model_ref,
            &reasoning,
            self.runtime.model_catalog().await,
        );
        let tools = self.runtime.tool_entries().await;
        let summary = ConfigSummary {
            display_text: render_config_summary(
                &config,
                self.config_path.as_deref(),
                &self.cwd,
                mode,
                &tools,
                module_epoch,
            ),
            config_path: self.config_path.as_ref().map(|p| p.display().to_string()),
            config_files: config_files(self.config_path.as_deref())
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            cwd: self.cwd.display().to_string(),
            session_dir: self.runtime.session_dir().map(|p| p.display().to_string()),
            profile: config.profile.name.clone(),
            model: ConfigModel {
                provider: model_ref.provider.clone(),
                name: model_ref.model.clone(),
                label: format!("{}/{}", model_ref.provider, model_ref.model),
            },
            model_options: selection.models,
            model_catalog_error: selection.error,
            reasoning: ConfigReasoning {
                enabled: reasoning.is_enabled(),
                effort: reasoning.effort,
                effort_options: selection.efforts,
                summary: reasoning.summary,
                budget_tokens: reasoning.budget_tokens,
            },
            permission_mode: format!("{mode:?}"),
            module_epoch: module_epoch.as_u64(),
            modules: config
                .modules
                .iter()
                .map(|(slot, id)| ConfigModule {
                    slot: slot.as_str().into(),
                    id: id.into(),
                })
                .collect(),
            tools_enabled: config.tools.enabled.clone(),
            registered_tools: tools
                .iter()
                .map(|(source, spec)| ConfigTool {
                    name: spec.name.clone(),
                    source: source.label(),
                    safety: format!("{:?}", spec.safety),
                    supports_parallel_tool_calls: spec.supports_parallel_tool_calls,
                    description: spec.description.clone(),
                })
                .collect(),
            components: config
                .components
                .iter()
                .map(|(id, component)| ConfigComponent {
                    id: id.clone(),
                    exports: component
                        .exports()
                        .map(|(slot, module_id, _)| ConfigComponentExport {
                            slot: slot.to_string(),
                            module_id: module_id.to_owned(),
                        })
                        .collect(),
                })
                .collect(),
            activity: None,
        };
        serde_json::to_value(summary).expect("config summary JSON")
    }
}
