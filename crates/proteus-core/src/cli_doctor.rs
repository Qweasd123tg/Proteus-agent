use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Result, bail};
use proteus_contracts::contracts::ToolRegistry;
use proteus_core::core::{
    AppConfig, AssemblyCheckSeverity, AssemblyPlan, ConfiguredToolExecutorConfig, ModuleCatalog,
    event_log_path,
};
use proteus_process_host::ProcessSpec;
use serde_json::Value;

use crate::cli_init::{mixed_config_files_warning, single_config_file_for_warning};

mod session_storage;

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

    let catalog = match ModuleCatalog::from_config(&config) {
        Ok(catalog) => catalog,
        Err(error) => {
            findings.error(format!("process module catalog: {error:#}"));
            findings.print();
            bail!("doctor found errors");
        }
    };

    let plan = match AssemblyPlan::resolve(
        config.clone(),
        effective_config,
        cwd.to_path_buf(),
        &catalog,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            findings.error(format!("assembly plan: {error:#}"));
            findings.print();
            bail!("doctor found errors");
        }
    };

    check_assembly_plan(&mut findings, &plan);
    check_model_config(&mut findings, &config);
    check_external_commands(&mut findings, &config, cwd);
    check_runtime_limits(&mut findings, &config);
    check_filesystem_paths(&mut findings, &config, cwd, effective_config);
    session_storage::check_session_storage(&mut findings, effective_config);

    if plan.is_valid() {
        match super::build_tool_registry_for_listing(&plan, &catalog) {
            Ok(registry) => {
                findings.ok(format!("tool registry: {} tools", registry.entries().len()));
                check_module_config_tool_references(&mut findings, &config, &registry);
            }
            Err(error) => findings.error(format!("tool registry failed: {error:#}")),
        }
    } else {
        findings.warn("tool registry skipped because the assembly plan is blocked");
    }

    findings.print();
    if findings.has_errors() {
        bail!("doctor found errors");
    }
    Ok(())
}

pub(crate) fn check_model_config(findings: &mut DoctorFindings, config: &AppConfig) {
    let model = match config.active_model_config() {
        Ok(model) => model,
        Err(error) => {
            findings.error(format!("model config: {error:#}"));
            return;
        }
    };

    findings.ok(format!("model: {}/{}", model.provider, model.model));
    findings.ok("model credentials and endpoint are owned by the selected component");
}

fn check_assembly_plan(findings: &mut DoctorFindings, plan: &AssemblyPlan) {
    let selected = plan
        .slots
        .iter()
        .filter(|slot| slot.module_id.is_some())
        .count();
    findings.ok(format!(
        "assembly plan: {} selected slots, {} components",
        selected,
        plan.components.len()
    ));
    for slot in &plan.slots {
        if let Some(module_id) = &slot.module_id {
            findings.ok(format!("module {}: {module_id}", slot.id));
        }
    }
    for check in &plan.checks {
        match check.severity {
            AssemblyCheckSeverity::Warning => {
                findings.warn(format!("assembly [{}]: {}", check.code, check.message));
            }
            AssemblyCheckSeverity::Error => {
                findings.error(format!("assembly [{}]: {}", check.code, check.message));
            }
        }
    }
}

/// Ключи-списки имён tools внутри opaque `module_config.*` (policy allow/deny
/// списки, tool exposure hot set, opencode permission groups). Core не знает
/// схему module config-ов, но эти ключи — известные межпаковые contracts
/// (см. docs/architecture/pack-contracts.md), и опечатка в имени tool-а иначе остаётся
/// молчаливо мёртвой записью.
const MODULE_CONFIG_TOOL_LIST_KEYS: [&str; 6] = [
    "allow",
    "allow_sandboxed",
    "ask_before",
    "deny",
    "always_include",
    "tools",
];

pub(crate) fn check_module_config_tool_references(
    findings: &mut DoctorFindings,
    config: &AppConfig,
    registry: &ToolRegistry,
) {
    let known = registry
        .entries()
        .into_iter()
        .map(|(_source, spec)| spec.name)
        .collect::<std::collections::HashSet<_>>();
    let mcp_servers = config
        .tools
        .mcp_servers
        .iter()
        .map(|server| server.name.as_str())
        .collect::<std::collections::HashSet<_>>();

    let mut checked = 0usize;
    let mut unknown = Vec::new();
    for (slot, modules) in &config.module_config {
        for (module_id, module_value) in modules {
            collect_unknown_tool_references(
                module_value,
                &format!("module_config.{slot}.{module_id}"),
                &known,
                &mcp_servers,
                &mut checked,
                &mut unknown,
            );
        }
    }

    if unknown.is_empty() {
        if checked > 0 {
            findings.ok(format!("module_config tool references: {checked} resolved"));
        }
        return;
    }
    for message in unknown {
        findings.warn(message);
    }
}

fn collect_unknown_tool_references(
    value: &Value,
    path: &str,
    known: &std::collections::HashSet<String>,
    mcp_servers: &std::collections::HashSet<&str>,
    checked: &mut usize,
    unknown: &mut Vec<String>,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    for (key, child) in object {
        if MODULE_CONFIG_TOOL_LIST_KEYS.contains(&key.as_str()) {
            let Some(names) = child.as_array() else {
                continue;
            };
            for name in names.iter().filter_map(Value::as_str) {
                *checked += 1;
                if known.contains(name) {
                    continue;
                }
                // MCP tools `<server>__<tool>` появляются после discovery;
                // достаточно, что server сконфигурирован.
                if name
                    .split_once("__")
                    .is_some_and(|(server, _)| mcp_servers.contains(server))
                {
                    continue;
                }
                unknown.push(format!("{path}.{key} references unknown tool '{name}'"));
            }
            continue;
        }
        collect_unknown_tool_references(
            child,
            &format!("{path}.{key}"),
            known,
            mcp_servers,
            checked,
            unknown,
        );
    }
}

pub(crate) fn check_external_commands(
    findings: &mut DoctorFindings,
    config: &AppConfig,
    cwd: &Path,
) {
    for (component_id, component) in &config.components {
        let process = component.process_spec(cwd).and_then(|spec| {
            spec.resolved_environment()?;
            Ok(spec)
        });
        match process {
            Ok(spec) => check_command(
                findings,
                &spec.command,
                spec.cwd.as_deref().unwrap_or(cwd),
                &format!(
                    "process component {} ({} exports)",
                    component_id,
                    component.exports().count()
                ),
            ),
            Err(error) => findings.error(format!(
                "process component {component_id} config: {error:#}"
            )),
        }
    }

    for tool in &config.tools.configured {
        match &tool.executor {
            ConfiguredToolExecutorConfig::Process {
                command,
                args,
                environment,
            } => {
                check_command(
                    findings,
                    command,
                    cwd,
                    &format!("configured process tool '{}'", tool.name),
                );
                let spec = ProcessSpec::new(command.clone())
                    .args(args.clone())
                    .env_allowlist(environment.env_allowlist.clone())
                    .envs(environment.env.clone());
                if let Err(error) = spec.resolved_environment() {
                    findings.error(format!(
                        "configured process tool '{}' environment: {error:#}",
                        tool.name
                    ));
                }
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

    let event_log_path = event_log_path(&config.event_log.path, config_path, cwd);
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
