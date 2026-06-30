use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Result, bail};
use proteus_core::{
    core::{AppConfig, BuiltinModuleCatalog, ConfiguredToolExecutorConfig, expand_user_path},
    domain::ModuleKind,
};
use serde_json::Value;

use crate::cli_init::{
    is_config_file_path, mixed_config_files_warning, single_config_file_for_warning,
};

pub(crate) async fn run_doctor(
    explicit_config: Option<&std::path::Path>,
    effective_config: Option<&std::path::Path>,
    cwd: &std::path::Path,
) -> Result<()> {
    let mut findings = DoctorFindings::default();
    findings.ok(format!("cwd: {}", cwd.display()));

    match effective_config {
        Some(path) if path.exists() => {
            let source = if explicit_config.is_some() {
                "explicit"
            } else {
                "default"
            };
            findings.ok(format!("config ({source}): {}", path.display()));
        }
        Some(path) => findings.warn(format!(
            "config path does not exist, defaults will be used: {}",
            path.display()
        )),
        None => findings.warn("no config path could be resolved; defaults will be used"),
    }

    if let Some(root) = config_root_for_doctor(effective_config) {
        let legacy_json = root.join("config.json");
        if legacy_json.exists() {
            findings.warn(format!(
                "legacy config.json is not used when configs/ exists: {}",
                legacy_json.display()
            ));
        }
    }
    if let Some(path) = single_config_file_for_warning(effective_config)
        && let Some(warning) = mixed_config_files_warning(&path)
    {
        findings.warn(warning);
    }

    let config = match AppConfig::load(explicit_config).await {
        Ok(config) => {
            findings.ok("config loaded");
            config
        }
        Err(error) => {
            findings.error(format!("config failed to load: {error:#}"));
            findings.print();
            bail!("doctor found errors");
        }
    };

    let mut catalog = BuiltinModuleCatalog::new();
    let plugin_reports = proteus_core::core::default_plugins_dir()
        .map(|plugins_dir| {
            findings.ok(format!("plugins dir: {}", plugins_dir.display()));
            proteus_core::core::load_plugins_from_dir(&plugins_dir, &mut catalog)
        })
        .unwrap_or_else(|| {
            findings.warn("plugins dir could not be resolved");
            Vec::new()
        });

    if plugin_reports.is_empty() {
        findings.warn("no plugins discovered");
    }
    for report in &plugin_reports {
        match &report.result {
            Ok(info) => findings.ok(format!("plugin loaded: {}", info.name)),
            Err(error) => findings.error(format!(
                "plugin failed: {}: {}",
                report.path.display(),
                super::first_line(&error.to_string())
            )),
        }
    }

    check_model_config(&mut findings, &catalog, &config);
    check_selected_modules(&mut findings, &catalog, &config);
    check_configured_tools(&mut findings, &config);
    check_external_commands(&mut findings, &config, cwd);
    check_runtime_limits(&mut findings, &config);
    check_filesystem_paths(&mut findings, &config, cwd, effective_config);

    match super::build_tool_registry_for_listing(&config, cwd) {
        Ok(registry) => findings.ok(format!("tool registry: {} tools", registry.entries().len())),
        Err(error) => findings.error(format!("tool registry failed: {error:#}")),
    }

    findings.print();
    if findings.has_errors() {
        bail!("doctor found errors");
    }
    Ok(())
}

pub(crate) fn check_model_config(
    findings: &mut DoctorFindings,
    catalog: &BuiltinModuleCatalog,
    config: &AppConfig,
) {
    let model = match config.active_model_config() {
        Ok(model) => model,
        Err(error) => {
            findings.error(format!("model config: {error:#}"));
            return;
        }
    };

    findings.ok(format!("model: {}/{}", model.provider, model.model));
    if catalog
        .manifest(ModuleKind::Model, model.provider.as_str())
        .is_some()
    {
        findings.ok(format!("module model: {}", model.provider));
    } else {
        findings.error(format!(
            "module model is not registered: {}",
            model.provider
        ));
    }

    check_model_secret(findings, &model);
}

pub(crate) fn check_model_secret(
    findings: &mut DoctorFindings,
    model: &proteus_core::core::ModelConfig,
) {
    let Some((default_env, json_key)) = provider_secret_defaults(&model.provider) else {
        if model.provider == "fake" {
            findings.ok("model secret: not required for fake provider");
        } else {
            findings.warn(format!(
                "model secret: no built-in secret check for provider '{}'",
                model.provider
            ));
        }
        return;
    };

    let Some(provider_config) = model.provider_config.as_object() else {
        check_env_secret(findings, default_env);
        return;
    };

    if provider_config
        .get("api_key")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        findings.warn("model secret: inline api_key configured; env/file is safer");
    } else if let Some(path) = provider_config.get("api_key_file").and_then(Value::as_str) {
        let path = expand_user_path(path);
        let key = provider_config
            .get("api_key_json_key")
            .and_then(Value::as_str)
            .unwrap_or(json_key);
        if path.exists() {
            findings.ok(format!(
                "model secret: api_key_file {} key '{}'",
                path.display(),
                key
            ));
        } else {
            findings.error(format!("model secret file is missing: {}", path.display()));
        }
    } else {
        let env_name = provider_config
            .get("api_key_env")
            .and_then(Value::as_str)
            .unwrap_or(default_env);
        check_env_secret(findings, env_name);
    }

    check_model_base_url(findings, &model.provider, provider_config);
}

fn check_model_base_url(
    findings: &mut DoctorFindings,
    provider: &str,
    provider_config: &serde_json::Map<String, Value>,
) {
    if let Some(path) = provider_config.get("base_url_file").and_then(Value::as_str) {
        let path = expand_user_path(path);
        let key = provider_config
            .get("base_url_json_key")
            .and_then(Value::as_str)
            .unwrap_or("base_url");
        if path.exists() {
            findings.ok(format!(
                "model endpoint: base_url_file {} key '{}'",
                path.display(),
                key
            ));
        } else {
            findings.error(format!(
                "model endpoint secret file is missing: {}",
                path.display()
            ));
        }
        return;
    }

    if let Some(env_name) = provider_config.get("base_url_env").and_then(Value::as_str) {
        match std::env::var(env_name) {
            Ok(value) if !value.trim().is_empty() => {
                findings.ok(format!("model endpoint: env {env_name} is set"));
            }
            _ => findings.error(format!(
                "model endpoint env var is missing or empty: {env_name}"
            )),
        }
        return;
    }

    if let Some(value) = provider_config.get("base_url").and_then(Value::as_str)
        && !is_public_default_base_url(provider, value)
    {
        findings.warn("model endpoint: inline custom base_url configured; file/env is safer if this URL is private");
    }
}

fn is_public_default_base_url(provider: &str, value: &str) -> bool {
    let value = value.trim_end_matches('/');
    matches!(
        (provider, value),
        ("anthropic", "https://api.anthropic.com")
            | ("openai", "https://api.openai.com/v1")
            | ("openai_compatible", "https://api.openai.com/v1")
    )
}

fn provider_secret_defaults(provider: &str) -> Option<(&'static str, &'static str)> {
    match provider {
        "anthropic" => Some(("ANTHROPIC_API_KEY", "anthropic_api_key")),
        "openai" | "openai_compatible" => Some(("OPENAI_API_KEY", "openai_api_key")),
        _ => None,
    }
}

fn check_env_secret(findings: &mut DoctorFindings, env_name: &str) {
    match std::env::var(env_name) {
        Ok(value) if !value.trim().is_empty() => {
            findings.ok(format!("model secret: env {env_name} is set"));
        }
        _ => findings.error(format!("model secret env is missing: {env_name}")),
    }
}

fn check_selected_modules(
    findings: &mut DoctorFindings,
    catalog: &BuiltinModuleCatalog,
    config: &AppConfig,
) {
    if config.modules.workflow == "single_loop" {
        findings.error("modules.workflow = \"single_loop\" is legacy; use \"coding.single_loop\"");
    }
    if config.modules.workflow == "plan_execute_review" {
        findings.error(
            "modules.workflow = \"plan_execute_review\" is legacy; use \"coding.plan_execute_review\"",
        );
    }

    let selected = [
        (ModuleKind::Search, config.modules.search.as_str()),
        (ModuleKind::Memory, config.modules.memory.as_str()),
        (
            ModuleKind::MemoryPolicy,
            config.modules.memory_policy.as_str(),
        ),
        (ModuleKind::Context, config.modules.context.as_str()),
        (ModuleKind::Policy, config.modules.policy.as_str()),
        (ModuleKind::Patch, config.modules.patch.as_str()),
        (ModuleKind::Compactor, config.modules.compactor.as_str()),
        (
            ModuleKind::ToolExposure,
            config.modules.tool_exposure.as_str(),
        ),
        (ModuleKind::Workflow, config.modules.workflow.as_str()),
        (ModuleKind::Renderer, config.modules.renderer.as_str()),
    ];

    for (kind, id) in selected {
        let label = super::module_kind_label(&kind);
        if catalog.manifest(kind.clone(), id).is_some() {
            findings.ok(format!("module {label}: {id}"));
        } else {
            findings.error(format!("module {label} is not registered: {id}"));
        }
    }
}

pub(crate) fn check_configured_tools(findings: &mut DoctorFindings, config: &AppConfig) {
    for tool in &config.tools.configured {
        if let ConfiguredToolExecutorConfig::Native { handler } = &tool.executor {
            match handler.as_str() {
                "apply_patch" | "search" => {}
                other => findings.error(format!(
                    "configured tool '{}' uses legacy native handler '{}'; use plugin tools.enabled instead",
                    tool.name, other
                )),
            }
        }
    }
}

fn check_external_commands(findings: &mut DoctorFindings, config: &AppConfig, cwd: &Path) {
    if config.modules.search == "rg" {
        check_command(findings, "rg", cwd, "search backend rg");
    }

    for tool in &config.tools.configured {
        match &tool.executor {
            ConfiguredToolExecutorConfig::Process { command, .. } => {
                check_command(
                    findings,
                    command,
                    cwd,
                    &format!("configured process tool '{}'", tool.name),
                );
            }
            ConfiguredToolExecutorConfig::Mcp { command, .. } => {
                check_command(
                    findings,
                    command,
                    cwd,
                    &format!("configured MCP tool '{}'", tool.name),
                );
            }
            ConfiguredToolExecutorConfig::Native { .. } => {}
        }
    }

    for server in &config.tools.mcp_servers {
        check_command(
            findings,
            &server.command,
            cwd,
            &format!("MCP server '{}'", server.name),
        );
    }
}

fn check_command(findings: &mut DoctorFindings, command: &str, cwd: &Path, label: &str) {
    if command_resolves(command, cwd) {
        findings.ok(format!("{label}: command available ({command})"));
    } else {
        findings.error(format!("{label}: command not found ({command})"));
    }
}

pub(crate) fn command_resolves(command: &str, cwd: &Path) -> bool {
    let command_path = Path::new(command);
    if command_path.is_absolute() {
        return command_path.exists();
    }
    if command.contains('/') || command.contains('\\') {
        return cwd.join(command_path).exists();
    }
    command_in_path(command)
}

fn command_in_path(command: &str) -> bool {
    if command.trim().is_empty() {
        return false;
    }
    if command == "rg" {
        return Command::new(command)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success());
    }

    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|path| path.join(command).exists())
}

fn check_runtime_limits(findings: &mut DoctorFindings, config: &AppConfig) {
    check_timeout_ms(
        findings,
        "runtime.model_timeout_ms",
        config.runtime.model_timeout_ms,
        120_000,
    );
    check_timeout_ms(
        findings,
        "runtime.context_timeout_ms",
        config.runtime.context_timeout_ms,
        10_000,
    );
    check_timeout_ms(
        findings,
        "runtime.workflow_timeout_ms",
        config.runtime.workflow_timeout_ms,
        300_000,
    );
    findings.ok(format!(
        "app_server.approval_timeout_ms: {}",
        format_timeout_ms(config.app_server.approval_timeout_ms)
    ));
}

pub(crate) fn check_timeout_ms(
    findings: &mut DoctorFindings,
    name: &str,
    value: u64,
    recommended_minimum: u64,
) {
    if value == 0 {
        findings.ok(format!("{name}: disabled"));
    } else if value < recommended_minimum {
        findings.warn(format!(
            "{name}: {} may be too low for real agents",
            format_timeout_ms(value)
        ));
    } else {
        findings.ok(format!("{name}: {}", format_timeout_ms(value)));
    }
}

pub(crate) fn format_timeout_ms(value: u64) -> String {
    if value == 0 {
        return "disabled".to_owned();
    }
    if value.is_multiple_of(3_600_000) {
        return format!("{}h", value / 3_600_000);
    }
    if value.is_multiple_of(60_000) {
        return format!("{}m", value / 60_000);
    }
    if value.is_multiple_of(1_000) {
        return format!("{}s", value / 1_000);
    }
    format!("{value}ms")
}

fn check_filesystem_paths(
    findings: &mut DoctorFindings,
    config: &AppConfig,
    cwd: &Path,
    config_path: Option<&std::path::Path>,
) {
    if cwd.is_dir() {
        findings.ok(format!("workspace dir exists: {}", cwd.display()));
    } else {
        findings.error(format!("workspace dir is missing: {}", cwd.display()));
    }

    let event_log_path =
        proteus_core::core::runtime::event_log_path(&config.event_log.path, config_path, cwd);
    match event_log_path.parent() {
        Some(parent) if parent.exists() => {
            if parent
                .metadata()
                .map(|metadata| metadata.permissions().readonly())
                .unwrap_or(false)
            {
                findings.error(format!(
                    "event log parent is read-only: {}",
                    parent.display()
                ));
            } else {
                findings.ok(format!("event log: {}", event_log_path.display()));
            }
        }
        Some(parent) => {
            if first_existing_ancestor(parent).is_some() {
                findings.warn(format!(
                    "event log parent will be created at runtime: {}",
                    parent.display()
                ));
            } else {
                findings.error(format!(
                    "event log parent has no existing ancestor: {}",
                    parent.display()
                ));
            }
        }
        None => findings.warn(format!(
            "event log path has no parent: {}",
            event_log_path.display()
        )),
    }
}

fn first_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = path;
    loop {
        if current.exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

pub(crate) fn config_root_for_doctor(config_path: Option<&std::path::Path>) -> Option<PathBuf> {
    let path = config_path?;
    if is_config_file_path(path) || path.is_file() {
        let parent = path.parent()?;
        if parent.file_name().and_then(|name| name.to_str()) == Some("configs") {
            return parent.parent().map(std::path::Path::to_path_buf);
        }
        return Some(parent.to_path_buf());
    }
    if path.file_name().and_then(|name| name.to_str()) == Some("configs") {
        return path.parent().map(std::path::Path::to_path_buf);
    }
    Some(path.to_path_buf())
}

#[derive(Default)]
pub(crate) struct DoctorFindings {
    pub(crate) entries: Vec<DoctorFinding>,
}

impl DoctorFindings {
    fn ok(&mut self, message: impl Into<String>) {
        self.entries.push(DoctorFinding::new("ok", message));
    }

    fn warn(&mut self, message: impl Into<String>) {
        self.entries.push(DoctorFinding::new("warn", message));
    }

    fn error(&mut self, message: impl Into<String>) {
        self.entries.push(DoctorFinding::new("error", message));
    }

    pub(crate) fn has_errors(&self) -> bool {
        self.entries.iter().any(|entry| entry.level == "error")
    }

    fn print(&self) {
        let rows = self
            .entries
            .iter()
            .map(|entry| [entry.level.to_owned(), entry.message.clone()])
            .collect::<Vec<_>>();
        println!("{}", super::render_table(["status", "check"], &rows));
    }
}

pub(crate) struct DoctorFinding {
    pub(crate) level: &'static str,
    pub(crate) message: String,
}

impl DoctorFinding {
    fn new(level: &'static str, message: impl Into<String>) -> Self {
        Self {
            level,
            message: message.into(),
        }
    }
}
