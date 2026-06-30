use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{
    contracts::ToolSource,
    core::{AppConfig, PluginLoadReport},
    domain::PermissionMode,
};

pub(super) fn render_config_summary(
    config: &AppConfig,
    config_path: Option<&Path>,
    cwd: &Path,
    mode: PermissionMode,
    tools: &[(ToolSource, crate::domain::ToolSpec)],
    plugin_reports: &[PluginLoadReport],
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
    lines.push(format!("  workflow: {}", config.modules.workflow));
    lines.push(format!("  context: {}", config.modules.context));
    lines.push(format!("  tool_exposure: {}", config.modules.tool_exposure));
    lines.push(format!("  policy: {}", config.modules.policy));
    lines.push(format!("  search: {}", config.modules.search));
    lines.push(format!("  patch: {}", config.modules.patch));
    lines.push(format!("  memory: {}", config.modules.memory));
    lines.push(format!("  memory_policy: {}", config.modules.memory_policy));
    lines.push(format!("  compactor: {}", config.modules.compactor));
    lines.push(format!("  renderer: {}", config.modules.renderer));

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

    lines.push("plugins:".to_owned());
    if plugin_reports.is_empty() {
        lines.push("  (none found)".to_owned());
    } else {
        for report in plugin_reports {
            let (name, version, description) = plugin_display_fields(report);
            let status = match &report.result {
                Ok(_) => "loaded".to_owned(),
                Err(error) => format!("error: {}", first_line(&error.to_string())),
            };
            if description.is_empty() {
                lines.push(format!("  - {name} {version}: {status}"));
            } else {
                lines.push(format!("  - {name} {version}: {status} - {description}"));
            }
        }
    }

    lines.join("\n")
}

pub(super) fn module_summary(config: &AppConfig) -> Vec<Value> {
    [
        ("workflow", config.modules.workflow.as_str()),
        ("context", config.modules.context.as_str()),
        ("tool_exposure", config.modules.tool_exposure.as_str()),
        ("policy", config.modules.policy.as_str()),
        ("search", config.modules.search.as_str()),
        ("patch", config.modules.patch.as_str()),
        ("memory", config.modules.memory.as_str()),
        ("memory_policy", config.modules.memory_policy.as_str()),
        ("compactor", config.modules.compactor.as_str()),
        ("renderer", config.modules.renderer.as_str()),
    ]
    .into_iter()
    .map(|(slot, id)| json!({ "slot": slot, "id": id }))
    .collect()
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
    }

    if looks_like_deepseek(config, active_model) {
        push_unique(&mut options, "high");
        push_unique(&mut options, "max");
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
    if let Some(active_provider) = config
        .active_provider
        .as_ref()
        .filter(|provider| !provider.trim().is_empty())
    {
        return config.providers.get(active_provider);
    }
    config.providers.get("default")
}

fn looks_like_deepseek(config: &AppConfig, active_model: &crate::domain::ModelRef) -> bool {
    let model = active_model.model.to_ascii_lowercase();
    let provider = active_model.provider.to_ascii_lowercase();
    let provider_config = config
        .active_model_config()
        .ok()
        .map(|model| model.provider_config.to_string().to_ascii_lowercase())
        .unwrap_or_default();
    model.contains("deepseek")
        || provider.contains("deepseek")
        || provider_config.contains("deepseek")
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

pub(super) fn plugin_summary(reports: &[PluginLoadReport]) -> Vec<Value> {
    reports
        .iter()
        .map(|report| {
            let (name, version, description) = plugin_display_fields(report);
            let status = match &report.result {
                Ok(_) => "loaded".to_owned(),
                Err(error) => format!("error: {}", first_line(&error.to_string())),
            };
            json!({
                "name": name,
                "version": version,
                "status": status,
                "description": description,
            })
        })
        .collect()
}

fn plugin_display_fields(report: &PluginLoadReport) -> (String, String, String) {
    match report.manifest.as_ref() {
        Some(manifest) => (
            manifest.name.clone(),
            manifest.version.clone(),
            manifest.description.clone().unwrap_or_default(),
        ),
        None => match report.result.as_ref() {
            Ok(info) => (info.name.clone(), "-".to_owned(), info.description.clone()),
            Err(_) => (
                report
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| report.path.display().to_string()),
                "-".to_owned(),
                String::new(),
            ),
        },
    }
}

fn first_line(text: &str) -> String {
    let mut lines = text.lines();
    let head = lines.next().unwrap_or("").trim_end().to_owned();
    if lines.next().is_some() {
        format!("{head} ...")
    } else {
        head
    }
}
