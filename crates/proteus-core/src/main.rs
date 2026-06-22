use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use async_trait::async_trait;
use clap::{Parser, ValueEnum};
use proteus_core::app_server::{
    http::{HttpServerConfig, run_http_app_server},
    stdio::run_stdio_app_server,
};
use proteus_core::domain::{
    AgentOutput, ModuleKind, ModuleManifest, PermissionMode, ToolSafety, new_thread_id,
};
use proteus_core::{
    contracts::{ApprovalRequest, ApprovalResponse, ApprovalTransport},
    core::{
        AgentRuntime, AppConfig, BuiltinModuleCatalog, ConfiguredToolExecutorConfig,
        ModuleBuildContext, ModuleEpoch, TopologyBuildInput, TopologyWarning,
        build_topology_snapshot, normalize_session_dir_path, render_topology_map,
        render_topology_markdown, render_topology_mermaid, render_topology_runtime_mermaid,
        render_topology_runtime_path, render_topology_table, session_id_from_session_dir,
    },
};
use serde_json::Value;
use tokio::time::sleep;

const CODING_PROFILE_CONFIG: &str = include_str!("../../../proteus.coding.example.toml");
const CODEX_PROFILE_CONFIG: &str = include_str!("../../../codex.config.toml");
const PROVIDER_PROFILE_CONFIG: &str = include_str!("../../../proteus.provider.example.toml");
const SAFE_PROFILE_CONFIG: &str = include_str!("../../../proteus.example.toml");
const INIT_CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Parser)]
#[command(author, version, about = "CLI-first Proteus skeleton")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long)]
    resume_session: Option<PathBuf>,
    #[arg(short, long)]
    interactive: bool,
    #[arg(long)]
    plan: bool,
    #[arg(long = "auto")]
    auto_mode: bool,
    #[arg(long, value_enum)]
    permission_mode: Option<CliPermissionMode>,
    #[arg(trailing_var_arg = true)]
    task: Vec<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliPermissionMode {
    Plan,
    Normal,
    Auto,
}

impl From<CliPermissionMode> for PermissionMode {
    fn from(value: CliPermissionMode) -> Self {
        match value {
            CliPermissionMode::Plan => Self::Plan,
            CliPermissionMode::Normal => Self::Normal,
            CliPermissionMode::Auto => Self::Auto,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(profile) = parse_init_command(&cli.task)? {
        return run_init(profile, cli.config.as_deref());
    }
    if is_modules_list_command(&cli.task) {
        let mut catalog = BuiltinModuleCatalog::new();
        let plugin_reports = proteus_core::core::default_plugins_dir()
            .map(|plugins_dir| {
                proteus_core::core::load_plugins_from_dir(&plugins_dir, &mut catalog)
            })
            .unwrap_or_default();
        println!("{}", render_module_list(&catalog.manifests()));
        if !plugin_reports.is_empty() {
            println!();
            println!("{}", render_plugin_list(&plugin_reports));
        }
        return Ok(());
    }
    if let Some(path) = parse_eval_report_command(&cli.task)? {
        let report = proteus_core::core::read_eval_report(path)?;
        println!("{}", render_eval_report(&report));
        return Ok(());
    }

    let config_path = AppConfig::resolve_config_path(cli.config.as_deref()).await?;
    let cwd = match cli.cwd {
        Some(ref cwd) => cwd.clone(),
        None => std::env::current_dir()?,
    };
    if is_doctor_command(&cli.task) {
        return run_doctor(cli.config.as_deref(), config_path.as_deref(), &cwd).await;
    }

    let mut config = AppConfig::load(cli.config.as_deref()).await?;
    config.permissions.mode = resolve_permission_mode(&cli, config.permissions.mode)?;
    if let Some(format) = parse_inspect_topology_command(&cli.task)? {
        let snapshot = build_cli_topology(
            &config,
            config_path.as_deref(),
            &cwd,
            config.permissions.mode,
        )?;
        println!("{}", render_inspect_topology(&snapshot, format)?);
        return Ok(());
    }
    if is_tools_list_command(&cli.task) {
        let registry = build_tool_registry_for_listing(&config, &cwd)?;
        println!("{}", render_tool_list(&registry));
        return Ok(());
    }
    if is_app_server_stdio_command(&cli.task) {
        return run_stdio_app_server(config, cwd, config_path, cli.resume_session).await;
    }
    if let Some(http_config) = parse_app_server_http_command(&cli.task)? {
        return run_http_app_server(config, cwd, config_path, cli.resume_session, http_config)
            .await;
    }
    if cli.interactive || cli.task.is_empty() {
        let runtime = build_cli_runtime(
            config.clone(),
            cwd.clone(),
            config_path.as_deref(),
            cli.resume_session.clone(),
        )?;
        return run_repl(runtime, config, cwd).await;
    }

    let runtime = build_cli_runtime(
        config.clone(),
        cwd.clone(),
        config_path.as_deref(),
        cli.resume_session.clone(),
    )?;
    let output = runtime.run(cli.task.join(" ")).await?;
    println!("{}", runtime.render(&output).await?);
    Ok(())
}

fn build_cli_runtime(
    config: AppConfig,
    cwd: PathBuf,
    config_path: Option<&std::path::Path>,
    resume_session: Option<PathBuf>,
) -> Result<AgentRuntime> {
    let mut builder = AgentRuntime::builder(config, cwd)
        .with_config_path(config_path)
        .with_approval(terminal_approval_transport());
    if let Some(session_dir) = resume_session {
        let session_dir = normalize_session_dir_path(session_dir)?;
        let session_id = session_id_from_session_dir(&session_dir)?;
        builder = builder.resume_from_session_dir(session_dir, session_id, new_thread_id());
    }
    builder.build()
}

fn is_modules_list_command(task: &[String]) -> bool {
    matches!(task, [module, command] if module == "modules" && command == "list")
}

fn parse_eval_report_command(task: &[String]) -> Result<Option<&str>> {
    match task {
        [namespace, command, path] if namespace == "eval" && command == "report" => Ok(Some(path)),
        [namespace, command, ..] if namespace == "eval" && command == "report" => {
            bail!("usage: proteus eval report <event-log-path>")
        }
        [namespace, ..] if namespace == "eval" => {
            bail!("usage: proteus eval report <event-log-path>")
        }
        _ => Ok(None),
    }
}

fn is_tools_list_command(task: &[String]) -> bool {
    matches!(task, [tool, command] if tool == "tools" && command == "list")
}

fn is_app_server_stdio_command(task: &[String]) -> bool {
    matches!(task, [server, transport] if server == "server" && transport == "stdio")
}

fn parse_app_server_http_command(task: &[String]) -> Result<Option<HttpServerConfig>> {
    let [server, transport, rest @ ..] = task else {
        return Ok(None);
    };
    if server != "server" || transport != "http" {
        return Ok(None);
    }

    let mut config = HttpServerConfig::default();
    let mut host = config.bind.ip();
    let mut port = config.bind.port();
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{}", app_server_http_usage()))?;
                host = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid --host value: {value}"))?;
            }
            "--port" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{}", app_server_http_usage()))?;
                port = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid --port value: {value}"))?;
            }
            "--token" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{}", app_server_http_usage()))?;
                if value.is_empty() {
                    bail!("--token must not be empty");
                }
                config.session_token = value.clone();
                config.require_session_token = true;
            }
            "--allow-origin" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{}", app_server_http_usage()))?;
                config.allowed_origins.push(value.clone());
            }
            _ => bail!("{}", app_server_http_usage()),
        }
    }
    config.bind = std::net::SocketAddr::new(host, port);
    Ok(Some(config))
}

fn app_server_http_usage() -> &'static str {
    "usage: proteus server http [--host <ip>] [--port <port>] [--token <token>] [--allow-origin <origin>]"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InspectTopologyFormat {
    Table,
    Json,
    Markdown,
    Runtime,
    RuntimeMermaid,
    Map,
    Mermaid,
}

fn parse_inspect_topology_command(task: &[String]) -> Result<Option<InspectTopologyFormat>> {
    let [namespace, rest @ ..] = task else {
        return Ok(None);
    };
    if namespace != "inspect" {
        return Ok(None);
    }

    match rest {
        [] => Ok(Some(InspectTopologyFormat::Markdown)),
        [command, args @ ..] if command == "topology" => {
            Ok(Some(parse_inspect_topology_format(args)?))
        }
        _ => bail!("{}", inspect_topology_usage()),
    }
}

fn parse_inspect_topology_format(args: &[String]) -> Result<InspectTopologyFormat> {
    let mut format = InspectTopologyFormat::Markdown;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--format" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{}", inspect_topology_usage()))?;
                format = inspect_topology_format_value(value)?;
            }
            value if value.starts_with("--format=") => {
                let value = value
                    .strip_prefix("--format=")
                    .expect("starts_with checked");
                format = inspect_topology_format_value(value)?;
            }
            _ => bail!("{}", inspect_topology_usage()),
        }
    }
    Ok(format)
}

fn inspect_topology_format_value(value: &str) -> Result<InspectTopologyFormat> {
    match value {
        "table" => Ok(InspectTopologyFormat::Table),
        "json" => Ok(InspectTopologyFormat::Json),
        "markdown" | "md" => Ok(InspectTopologyFormat::Markdown),
        "runtime" | "path" => Ok(InspectTopologyFormat::Runtime),
        "runtime-mermaid" | "runtime_mmd" | "runtime-mmd" => {
            Ok(InspectTopologyFormat::RuntimeMermaid)
        }
        "map" => Ok(InspectTopologyFormat::Map),
        "mermaid" | "mmd" => Ok(InspectTopologyFormat::Mermaid),
        _ => bail!(
            "unknown topology format '{value}', expected table, json, markdown, runtime, runtime-mermaid, map, or mermaid"
        ),
    }
}

fn inspect_topology_usage() -> &'static str {
    "usage: proteus inspect [topology] [--format table|json|markdown|runtime|runtime-mermaid|map|mermaid]"
}

fn is_doctor_command(task: &[String]) -> bool {
    matches!(task, [command] if command == "doctor")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitProfile {
    Coding,
    Codex,
    Full,
    Safe,
}

impl InitProfile {
    fn config_name(self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::Codex => "codex",
            Self::Full => "full",
            Self::Safe => "safe",
        }
    }

    fn config_body(self) -> &'static str {
        match self {
            Self::Coding | Self::Full => CODING_PROFILE_CONFIG,
            Self::Codex => CODEX_PROFILE_CONFIG,
            Self::Safe => SAFE_PROFILE_CONFIG,
        }
    }
}

fn parse_init_command(task: &[String]) -> Result<Option<InitProfile>> {
    match task {
        [command] if command == "init" => Ok(Some(InitProfile::Coding)),
        [command, profile] if command == "init" => match profile.as_str() {
            "coding" => Ok(Some(InitProfile::Coding)),
            "codex" => Ok(Some(InitProfile::Codex)),
            "full" => Ok(Some(InitProfile::Full)),
            "safe" => Ok(Some(InitProfile::Safe)),
            other => bail!("unknown init profile '{other}', expected coding, codex, full, or safe"),
        },
        [command, ..] if command == "init" => {
            bail!("usage: proteus init [coding|codex|full|safe]")
        }
        _ => Ok(None),
    }
}

fn run_init(profile: InitProfile, explicit_config: Option<&Path>) -> Result<()> {
    let config_path = explicit_config
        .map(init_config_path_from_arg)
        .or_else(AppConfig::default_user_config_path)
        .ok_or_else(|| anyhow::anyhow!("could not resolve default config path"))?;
    let destination = init_destination_path(&config_path, profile);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&destination, profile.config_body_for_init())?;

    println!(
        "Initialized {} profile: {}",
        profile.config_name(),
        destination.display()
    );
    if let Some(warning) = mixed_config_files_warning(&destination) {
        println!("warning: {warning}");
    }
    println!("Next: proteus doctor");
    Ok(())
}

fn init_config_path_from_arg(path: &Path) -> PathBuf {
    AppConfig::named_config_destination_path(path).unwrap_or_else(|| path.to_path_buf())
}

impl InitProfile {
    fn config_body_for_init(self) -> String {
        match self {
            Self::Coding | Self::Codex | Self::Full => {
                let profile_body = strip_profile_include(self.config_body()).trim_start();
                format!("{}\n\n{}", PROVIDER_PROFILE_CONFIG.trim_end(), profile_body)
            }
            Self::Safe => self.config_body().to_owned(),
        }
    }
}

fn strip_profile_include(config: &str) -> &str {
    if let Some(rest) = config.strip_prefix("include = \"proteus.provider.example.toml\"") {
        rest
    } else {
        config
    }
}

fn init_destination_path(config_path: &Path, _profile: InitProfile) -> PathBuf {
    if is_config_file_path(config_path) {
        config_path.to_path_buf()
    } else {
        config_path.join(INIT_CONFIG_FILE)
    }
}

fn is_config_file_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("toml" | "json")
    )
}

fn single_config_file_for_warning(config_path: Option<&Path>) -> Option<PathBuf> {
    let path = config_path?;
    if is_config_file_path(path) {
        return (path.file_name().and_then(|name| name.to_str()) == Some(INIT_CONFIG_FILE))
            .then(|| path.to_path_buf());
    }
    Some(path.join(INIT_CONFIG_FILE)).filter(|path| path.exists())
}

fn mixed_config_files_warning(config_file: &Path) -> Option<String> {
    if config_file.file_name().and_then(|name| name.to_str()) != Some(INIT_CONFIG_FILE) {
        return None;
    }
    let siblings = sibling_config_files(config_file);
    if siblings.is_empty() {
        return None;
    }
    Some(format!(
        "config dir also contains {}. Proteus loads every .toml/.json file when given the directory; move old files away or pass --config {} to load only this file.",
        siblings
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        config_file.display()
    ))
}

fn sibling_config_files(config_file: &Path) -> Vec<PathBuf> {
    let Some(parent) = config_file.parent() else {
        return Vec::new();
    };
    let config_name = config_file.file_name();
    let mut files = std::fs::read_dir(parent)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && is_config_file_path(path) && path.file_name() != config_name
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

async fn run_doctor(
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
                first_line(&error.to_string())
            )),
        }
    }

    check_model_config(&mut findings, &catalog, &config);
    check_selected_modules(&mut findings, &catalog, &config);
    check_configured_tools(&mut findings, &config);
    check_external_commands(&mut findings, &config, cwd);
    check_runtime_limits(&mut findings, &config);
    check_filesystem_paths(&mut findings, &config, cwd, effective_config);

    match build_tool_registry_for_listing(&config, cwd) {
        Ok(registry) => findings.ok(format!("tool registry: {} tools", registry.entries().len())),
        Err(error) => findings.error(format!("tool registry failed: {error:#}")),
    }

    findings.print();
    if findings.has_errors() {
        bail!("doctor found errors");
    }
    Ok(())
}

fn check_model_config(
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

fn check_model_secret(findings: &mut DoctorFindings, model: &proteus_core::core::ModelConfig) {
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
        return;
    }

    if let Some(path) = provider_config.get("api_key_file").and_then(Value::as_str) {
        let path = Path::new(path);
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
        return;
    }

    let env_name = provider_config
        .get("api_key_env")
        .and_then(Value::as_str)
        .unwrap_or(default_env);
    check_env_secret(findings, env_name);
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
        let label = module_kind_label(&kind);
        if catalog.manifest(kind.clone(), id).is_some() {
            findings.ok(format!("module {label}: {id}"));
        } else {
            findings.error(format!("module {label} is not registered: {id}"));
        }
    }
}

fn check_configured_tools(findings: &mut DoctorFindings, config: &AppConfig) {
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

fn command_resolves(command: &str, cwd: &Path) -> bool {
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

fn check_timeout_ms(
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

fn format_timeout_ms(value: u64) -> String {
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

fn config_root_for_doctor(config_path: Option<&std::path::Path>) -> Option<PathBuf> {
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
struct DoctorFindings {
    entries: Vec<DoctorFinding>,
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

    fn has_errors(&self) -> bool {
        self.entries.iter().any(|entry| entry.level == "error")
    }

    fn print(&self) {
        let rows = self
            .entries
            .iter()
            .map(|entry| [entry.level.to_owned(), entry.message.clone()])
            .collect::<Vec<_>>();
        println!("{}", render_table(["status", "check"], &rows));
    }
}

struct DoctorFinding {
    level: &'static str,
    message: String,
}

impl DoctorFinding {
    fn new(level: &'static str, message: impl Into<String>) -> Self {
        Self {
            level,
            message: message.into(),
        }
    }
}

fn render_module_list(manifests: &[ModuleManifest]) -> String {
    let rows = manifests
        .iter()
        .map(|manifest| {
            [
                module_kind_label(&manifest.kind).to_owned(),
                manifest.id.clone(),
                manifest.capabilities.join(","),
                manifest.description.clone().unwrap_or_default(),
            ]
        })
        .collect::<Vec<_>>();

    render_table(["kind", "id", "capabilities", "description"], &rows)
}

fn render_plugin_list(reports: &[proteus_core::core::PluginLoadReport]) -> String {
    let rows = reports
        .iter()
        .map(|report| {
            let (name, version, description) = match report.manifest.as_ref() {
                Some(manifest) => (
                    manifest.name.clone(),
                    manifest.version.clone(),
                    manifest.description.clone().unwrap_or_default(),
                ),
                None => match report.result.as_ref() {
                    Ok(info) => (info.name.clone(), "-".to_owned(), info.description.clone()),
                    // Нет ни manifest'а, ни загруженного info — fallback на путь.
                    Err(_) => (
                        report
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| report.path.display().to_string()),
                        "-".to_owned(),
                        String::new(),
                    ),
                },
            };
            let status = match &report.result {
                Ok(_) => "loaded".to_owned(),
                Err(error) => format!("error: {}", first_line(&error.to_string())),
            };
            [name, version, status, description]
        })
        .collect::<Vec<_>>();

    let mut out = String::from("Plugins:\n");
    out.push_str(&render_table(
        ["name", "version", "status", "description"],
        &rows,
    ));
    out
}

/// Сжимает многострочный текст в первую строку, добавляя " …" если
/// были ещё строки. Нужно для table-рендеринга: `toml` parser возвращает
/// многострочный message с caret'ами, который ломает колоночное
/// выравнивание.
fn first_line(text: &str) -> String {
    let mut lines = text.lines();
    let head = lines.next().unwrap_or("").trim_end().to_owned();
    if lines.next().is_some() {
        format!("{head} …")
    } else {
        head
    }
}

fn build_tool_registry_for_listing(
    config: &AppConfig,
    cwd: &std::path::Path,
) -> Result<proteus_core::contracts::ToolRegistry> {
    let mut catalog = BuiltinModuleCatalog::new();
    if let Some(plugins_dir) = proteus_core::core::default_plugins_dir() {
        let _ = proteus_core::core::load_plugins_from_dir(&plugins_dir, &mut catalog);
    }
    let build_ctx = ModuleBuildContext {
        config,
        cwd,
        context_providers: catalog.context_providers(),
    };
    let search = catalog.build_search(&config.modules.search, &build_ctx)?;
    let patch = catalog.build_patch(&config.modules.patch, &build_ctx)?;
    let memory = catalog.build_memory(&config.modules.memory, &build_ctx)?;
    catalog.build_tools(&build_ctx, search, patch, memory)
}

fn build_cli_topology(
    config: &AppConfig,
    config_path: Option<&std::path::Path>,
    cwd: &std::path::Path,
    permission_mode: PermissionMode,
) -> Result<proteus_core::core::TopologySnapshot> {
    let mut catalog = BuiltinModuleCatalog::new();
    let plugin_reports = proteus_core::core::default_plugins_dir()
        .map(|plugins_dir| proteus_core::core::load_plugins_from_dir(&plugins_dir, &mut catalog))
        .unwrap_or_default();
    let catalog_entries = catalog.entry_summaries();
    let build_ctx = ModuleBuildContext {
        config,
        cwd,
        context_providers: catalog.context_providers(),
    };
    let mut extra_warnings = Vec::new();
    let search = match catalog.build_search(&config.modules.search, &build_ctx) {
        Ok(search) => Some(search),
        Err(error) => {
            extra_warnings.push(TopologyWarning::error(format!(
                "inspect could not build search module {}: {error:#}",
                config.modules.search
            )));
            None
        }
    };
    let patch = match catalog.build_patch(&config.modules.patch, &build_ctx) {
        Ok(patch) => Some(patch),
        Err(error) => {
            extra_warnings.push(TopologyWarning::error(format!(
                "inspect could not build patch module {}: {error:#}",
                config.modules.patch
            )));
            None
        }
    };
    let memory = match catalog.build_memory(&config.modules.memory, &build_ctx) {
        Ok(memory) => Some(memory),
        Err(error) => {
            extra_warnings.push(TopologyWarning::error(format!(
                "inspect could not build memory module {}: {error:#}",
                config.modules.memory
            )));
            None
        }
    };
    let tool_entries = match (search, patch, memory) {
        (Some(search), Some(patch), Some(memory)) => {
            match catalog.build_tools(&build_ctx, search, patch, memory) {
                Ok(tools) => tools.entries(),
                Err(error) => {
                    extra_warnings.push(TopologyWarning::error(format!(
                        "inspect could not build ToolRegistry: {error:#}"
                    )));
                    Vec::new()
                }
            }
        }
        _ => Vec::new(),
    };

    Ok(build_topology_snapshot(TopologyBuildInput {
        config,
        config_path,
        cwd,
        catalog_entries: &catalog_entries,
        tools: &tool_entries,
        plugin_reports: &plugin_reports,
        module_epoch: ModuleEpoch::initial(),
        permission_mode,
        extra_warnings,
    }))
}

fn render_inspect_topology(
    snapshot: &proteus_core::core::TopologySnapshot,
    format: InspectTopologyFormat,
) -> Result<String> {
    match format {
        InspectTopologyFormat::Table => Ok(render_topology_table(snapshot)),
        InspectTopologyFormat::Json => serde_json::to_string_pretty(snapshot).map_err(Into::into),
        InspectTopologyFormat::Markdown => Ok(render_topology_markdown(snapshot)),
        InspectTopologyFormat::Runtime => Ok(render_topology_runtime_path(snapshot)),
        InspectTopologyFormat::RuntimeMermaid => Ok(render_topology_runtime_mermaid(snapshot)),
        InspectTopologyFormat::Map => Ok(render_topology_map(snapshot)),
        InspectTopologyFormat::Mermaid => Ok(render_topology_mermaid(snapshot)),
    }
}

fn render_tool_list(registry: &proteus_core::contracts::ToolRegistry) -> String {
    let rows = registry
        .entries()
        .into_iter()
        .map(|(source, spec)| {
            [
                spec.name,
                source.label(),
                tool_safety_label(&spec.safety).to_owned(),
                spec.timeout_ms
                    .map(|timeout| timeout.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                spec.description,
            ]
        })
        .collect::<Vec<_>>();

    render_table(
        ["name", "source", "safety", "timeout_ms", "description"],
        &rows,
    )
}

fn render_eval_report(report: &proteus_core::core::EvalReport) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Eval report: {}", report.event_log_path.display()));
    lines.push(format!(
        "Status: {}",
        if report.succeeded() {
            "success"
        } else {
            "failed"
        }
    ));
    lines.push(format!("Events: {}", report.events));
    lines.push(format!(
        "Turns: started={}, finished={}, failed={}",
        report.turns_started, report.turns_finished, report.turns_failed
    ));
    lines.push(format!(
        "Model calls: {}, tool calls: {} (failures={})",
        report.model_calls, report.tool_calls, report.tool_failures
    ));
    lines.push(format!(
        "Approvals: requested={}, resolved={}, approved={}, denied={}",
        report.approvals_requested,
        report.approvals_resolved,
        report.approvals_approved,
        report.approvals_denied
    ));
    lines.push(format!(
        "Tokens: estimated_input={}, provider_input={}, provider_output={}",
        report.estimated_input_tokens, report.provider_input_tokens, report.provider_output_tokens
    ));
    if let Some(duration_ms) = report.duration_ms {
        lines.push(format!("Duration: {duration_ms} ms"));
    }
    if report.changed_files.is_empty() {
        lines.push("Changed files: none".to_owned());
    } else {
        lines.push(format!(
            "Changed files: {}",
            report.changed_files.join(", ")
        ));
    }
    if let Some(reason) = &report.failure_reason {
        lines.push(format!("Failure reason: {reason}"));
    }
    lines.join("\n")
}

fn render_table<const N: usize>(headers: [&str; N], rows: &[[String; N]]) -> String {
    let mut widths = headers
        .iter()
        .map(|header| header.chars().count())
        .collect::<Vec<_>>();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    let mut rendered = String::new();
    rendered.push_str(&render_table_row(&headers.map(str::to_owned), &widths));
    rendered.push('\n');
    rendered.push_str(
        &widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("  "),
    );
    for row in rows {
        rendered.push('\n');
        rendered.push_str(&render_table_row(row, &widths));
    }
    rendered
}

fn render_table_row<const N: usize>(row: &[String; N], widths: &[usize]) -> String {
    row.iter()
        .enumerate()
        .map(|(index, cell)| format!("{cell:width$}", width = widths[index]))
        .collect::<Vec<_>>()
        .join("  ")
}

fn tool_safety_label(safety: &ToolSafety) -> &'static str {
    match safety {
        ToolSafety::ReadOnly => "ReadOnly",
        ToolSafety::WritesFiles => "WritesFiles",
        ToolSafety::RunsCommands => "RunsCommands",
        ToolSafety::Network => "Network",
        ToolSafety::Dangerous => "Dangerous",
        _ => "Unknown",
    }
}

fn module_kind_label(kind: &ModuleKind) -> &'static str {
    match kind {
        ModuleKind::Model => "model",
        ModuleKind::Search => "search",
        ModuleKind::Memory => "memory",
        ModuleKind::MemoryPolicy => "memory_policy",
        ModuleKind::Context => "context",
        ModuleKind::Tool => "tool",
        ModuleKind::Policy => "policy",
        ModuleKind::Patch => "patch",
        ModuleKind::Compactor => "compactor",
        ModuleKind::ToolExposure => "tool_exposure",
        ModuleKind::Workflow => "workflow",
        ModuleKind::Renderer => "renderer",
        _ => "unknown",
    }
}

fn resolve_permission_mode(cli: &Cli, configured: PermissionMode) -> Result<PermissionMode> {
    let selected = [
        cli.plan.then_some(PermissionMode::Plan),
        cli.auto_mode.then_some(PermissionMode::Auto),
        cli.permission_mode.map(Into::into),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    if selected.len() > 1 {
        bail!("use only one of --plan, --auto, or --permission-mode");
    }

    Ok(selected.into_iter().next().unwrap_or(configured))
}

fn terminal_approval_transport() -> Arc<dyn ApprovalTransport> {
    Arc::new(TerminalApprovalTransport {
        enabled: io::stdin().is_terminal() && io::stdout().is_terminal(),
    })
}

#[derive(Debug)]
struct TerminalApprovalTransport {
    enabled: bool,
}

#[async_trait]
impl ApprovalTransport for TerminalApprovalTransport {
    fn can_request_approval(&self) -> bool {
        self.enabled
    }

    async fn request_approval(&self, request: ApprovalRequest) -> Result<ApprovalResponse> {
        if !self.enabled {
            return Ok(ApprovalResponse::deny(format!(
                "approval transport is not interactive: {}",
                request.reason
            )));
        }

        let args = request.call.args.to_string();
        let args = if args.chars().count() > 500 {
            format!("{}...", args.chars().take(500).collect::<String>())
        } else {
            args
        };
        eprintln!();
        eprintln!("Approval requested");
        eprintln!("tool: {}", request.call.name);
        eprintln!("cwd: {}", request.cwd.display());
        eprintln!("reason: {}", request.reason);
        if let Some(spec) = &request.tool_spec {
            eprintln!("safety: {:?}", spec.safety);
        }
        eprintln!("args: {args}");
        eprint!("Approve this tool call? [y/N] ");
        io::stderr().flush()?;

        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        let approved = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        if approved {
            Ok(ApprovalResponse::approve())
        } else {
            Ok(ApprovalResponse::deny(format!(
                "tool call was not approved: {}",
                request.reason
            )))
        }
    }
}

/// Реализует slash-команду `/remember KIND TEXT` в REPL.
///
/// Парсинг: первое слово — `kind` (`preference` или `fact`). Остальное —
/// `content`. Если первое слово не валидный kind, всё идёт как `fact`
/// content. Это удобный shortcut: `/remember project uses pnpm` просто
/// работает как fact.
async fn handle_remember(runtime: &AgentRuntime, rest: &str) -> Result<String> {
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        bail!("usage: /remember [preference|fact] <content>");
    }
    let (kind, content) = match trimmed.split_once(char::is_whitespace) {
        Some((first, rest_content)) if matches!(first, "preference" | "fact") => {
            (first.to_owned(), rest_content.trim().to_owned())
        }
        _ => ("fact".to_owned(), trimmed.to_owned()),
    };
    if content.is_empty() {
        bail!("/remember: content is empty");
    }
    let item = proteus_core::domain::MemoryItem::new(&kind, &content, serde_json::Value::Null);
    runtime.memory().await.remember(item).await?;
    Ok(format!("stored ({kind}): {content}"))
}

async fn run_repl(runtime: AgentRuntime, config: AppConfig, cwd: PathBuf) -> Result<()> {
    println!("{}", repl_header(&config, &cwd, runtime.session_dir())?);
    let tty_composer = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mut footer = initial_footer(&config)?;

    loop {
        print_composer_prompt(&footer, tty_composer)?;

        let mut input = String::new();
        let bytes = io::stdin().read_line(&mut input)?;
        if tty_composer {
            clear_composer_footer()?;
        }
        if bytes == 0 {
            println!();
            break;
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        match input {
            "/exit" | "/quit" => break,
            "/clear" | "/reset" => {
                runtime.clear_history().await?;
                println!("{}", small_block("state", &["history cleared".to_owned()]));
                continue;
            }
            "/history" => {
                println!(
                    "{}",
                    small_block(
                        "history",
                        &[format!("messages: {}", runtime.history_len().await)]
                    )
                );
                continue;
            }
            "/help" => {
                println!(
                    "{}",
                    small_block(
                        "help",
                        &[
                            "/help            show this help".to_owned(),
                            "/history         show in-memory history size".to_owned(),
                            "/clear, /reset   clear in-memory history".to_owned(),
                            "/remember KIND TEXT  store KIND=preference|fact (KIND=fact if omitted)"
                                .to_owned(),
                            "/exit, /quit     leave the REPL".to_owned(),
                            "examples: read_file Cargo.toml | summarize project".to_owned(),
                        ],
                    )
                );
                continue;
            }
            _ => {}
        }

        if let Some(rest) = input.strip_prefix("/remember ").map(str::trim) {
            match handle_remember(&runtime, rest).await {
                Ok(message) => {
                    println!("{}", small_block("memory", &[message]));
                }
                Err(error) => {
                    eprintln!("error: {error:#}");
                }
            }
            continue;
        }

        match run_with_spinner(&runtime, input.to_owned(), tty_composer).await {
            Ok(output) => {
                print_assistant_output(&output.text, tty_composer).await?;
                footer = footer_from_output(&config, &output)?;
            }
            Err(error) => eprintln!("error: {error:#}"),
        }
    }

    Ok(())
}

fn repl_header(
    config: &AppConfig,
    cwd: &std::path::Path,
    session_dir: Option<&std::path::Path>,
) -> Result<String> {
    let model = config.active_model_config()?;
    let mut lines = vec![
        "Proteus REPL".to_owned(),
        "type a task, /help, or /exit".to_owned(),
        format!("profile: {}", config.profile.name),
        format!("model: {}/{}", model.provider, model.model),
        format!("cwd: {}", cwd.display()),
        format!(
            "modules: workflow={} context={} memory={} search={} renderer={}",
            config.modules.workflow,
            config.modules.context,
            config.modules.memory,
            config.modules.search,
            config.modules.renderer
        ),
        format!("tools: {}", config.tools.enabled.join(", ")),
    ];
    if let Some(session_dir) = session_dir {
        lines.push(format!("session: {}", session_dir.display()));
    }
    Ok(small_block("Proteus", &lines))
}

fn small_block(title: &str, lines: &[String]) -> String {
    let text_width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or_default()
        .max(72);
    let inner_width = text_width + 2;
    let title = format!(" {title} ");
    let right = inner_width.saturating_sub(title.chars().count());
    let mut rendered = format!("╭{}{}╮\n", title, "─".repeat(right));
    for line in lines {
        rendered.push_str(&format!(
            "│ {}{} │\n",
            line,
            " ".repeat(text_width.saturating_sub(line.chars().count()))
        ));
    }
    rendered.push_str(&format!("╰{}╯", "─".repeat(inner_width)));
    rendered
}

fn assistant_output(rendered: &str) -> String {
    match rendered.split_once('\n') {
        Some((first, rest)) => format!("● {first}\n{rest}"),
        None => format!("● {rendered}"),
    }
}

async fn run_with_spinner(
    runtime: &AgentRuntime,
    input: String,
    tty_composer: bool,
) -> Result<AgentOutput> {
    let run = runtime.run(input);
    tokio::pin!(run);

    if !tty_composer {
        return run.await;
    }

    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut frame = 0usize;
    loop {
        tokio::select! {
            result = &mut run => {
                clear_current_line()?;
                return result;
            }
            _ = sleep(Duration::from_millis(120)) => {
                print!("\r\x1b[2K{} thinking", frames[frame % frames.len()]);
                io::stdout().flush()?;
                frame += 1;
            }
        }
    }
}

async fn print_assistant_output(text: &str, tty_composer: bool) -> Result<()> {
    if !tty_composer {
        println!("{}", assistant_output(text));
        return Ok(());
    }

    print!("● ");
    io::stdout().flush()?;

    let char_count = text.chars().count();
    let batch_size = if char_count > 2_000 {
        32
    } else if char_count > 800 {
        16
    } else {
        8
    };

    let mut buffer = String::new();
    let mut buffered = 0usize;
    for ch in text.chars() {
        buffer.push(ch);
        buffered += 1;
        if buffered >= batch_size || ch == '\n' {
            print!("{buffer}");
            io::stdout().flush()?;
            buffer.clear();
            buffered = 0;
            sleep(Duration::from_millis(8)).await;
        }
    }
    if !buffer.is_empty() {
        print!("{buffer}");
    }
    println!();
    io::stdout().flush()?;
    Ok(())
}

fn print_composer_prompt(footer: &str, tty_composer: bool) -> Result<()> {
    if !tty_composer {
        print!("❯ ");
        io::stdout().flush()?;
        return Ok(());
    }

    let separator = "─".repeat(composer_width(footer));
    print!("❯ \n{separator}\n  {footer}\x1b[2A\r\x1b[2C");
    io::stdout().flush()?;
    Ok(())
}

fn clear_composer_footer() -> Result<()> {
    print!("\r\x1b[2K\x1b[1B\r\x1b[2K\x1b[1A\r");
    io::stdout().flush()?;
    Ok(())
}

fn clear_current_line() -> Result<()> {
    print!("\r\x1b[2K");
    io::stdout().flush()?;
    Ok(())
}

fn composer_width(footer: &str) -> usize {
    footer.chars().count().max(72)
}

fn initial_footer(config: &AppConfig) -> Result<String> {
    let model = config.active_model_config()?;
    Ok(format!(
        "? for shortcuts    model {}/{} · Context waiting",
        model.provider, model.model
    ))
}

fn footer_from_output(config: &AppConfig, output: &AgentOutput) -> Result<String> {
    let model = footer_model(config, output)?;
    let context = footer_context(output);
    let session = output
        .metadata
        .get("session_id")
        .and_then(Value::as_str)
        .map(short_id)
        .unwrap_or("unknown");
    Ok(format!(
        "? for shortcuts    {model} · {context} · session {session}"
    ))
}

fn footer_model(config: &AppConfig, output: &AgentOutput) -> Result<String> {
    if let Some(model) = output.metadata.get("model") {
        let provider = model.get("provider").and_then(Value::as_str);
        let name = model
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| model.get("model").and_then(Value::as_str));
        if let Some(name) = name {
            return Ok(match provider {
                Some(provider) if !provider.is_empty() => format!("model {provider}/{name}"),
                _ => format!("model {name}"),
            });
        }
    }

    let model = config.active_model_config()?;
    Ok(format!("model {}/{}", model.provider, model.model))
}

fn footer_context(output: &AgentOutput) -> String {
    let context = output.metadata.get("context");
    let tokens = context
        .and_then(|context| context.get("token_estimate"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let chunks = context
        .and_then(|context| context.get("chunks"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let max_tokens = 200_000_u64;
    let percent = ((tokens as f64 / max_tokens as f64) * 100.0).clamp(0.0, 100.0);
    let chunk_word = if chunks == 1 { "chunk" } else { "chunks" };
    format!(
        "Context {:.0}% · {} in · {} {}",
        percent, tokens, chunks, chunk_word
    )
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_core::domain::ModuleManifest;

    /// Disables plugin loading so tests don't pick up the developer's
    /// `~/.proteus/plugins/` contents. See also the same helper in the
    /// `module_swap` integration test.
    fn disable_plugins() {
        static DISABLE: std::sync::Once = std::sync::Once::new();
        DISABLE.call_once(|| unsafe {
            std::env::set_var("PROTEUS_PLUGINS_DISABLE", "1");
        });
    }

    #[test]
    fn modules_list_command_is_exact() {
        assert!(is_modules_list_command(&[
            "modules".to_owned(),
            "list".to_owned()
        ]));
        assert!(!is_modules_list_command(&["modules".to_owned()]));
        assert!(!is_modules_list_command(&[
            "modules".to_owned(),
            "list".to_owned(),
            "extra".to_owned()
        ]));
    }

    #[test]
    fn tools_list_command_is_exact() {
        assert!(is_tools_list_command(&[
            "tools".to_owned(),
            "list".to_owned()
        ]));
        assert!(!is_tools_list_command(&["tools".to_owned()]));
        assert!(!is_tools_list_command(&[
            "tools".to_owned(),
            "list".to_owned(),
            "extra".to_owned()
        ]));
    }

    #[test]
    fn inspect_topology_command_parses_default_and_formats() {
        assert_eq!(
            parse_inspect_topology_command(&["inspect".to_owned()])
                .expect("parse")
                .expect("inspect command"),
            InspectTopologyFormat::Markdown
        );
        assert_eq!(
            parse_inspect_topology_command(&[
                "inspect".to_owned(),
                "topology".to_owned(),
                "--format".to_owned(),
                "json".to_owned(),
            ])
            .expect("parse")
            .expect("inspect command"),
            InspectTopologyFormat::Json
        );
        assert_eq!(
            parse_inspect_topology_command(&[
                "inspect".to_owned(),
                "topology".to_owned(),
                "--format=map".to_owned(),
            ])
            .expect("parse")
            .expect("inspect command"),
            InspectTopologyFormat::Map
        );
        assert_eq!(
            parse_inspect_topology_command(&[
                "inspect".to_owned(),
                "topology".to_owned(),
                "--format=runtime".to_owned(),
            ])
            .expect("parse")
            .expect("inspect command"),
            InspectTopologyFormat::Runtime
        );
        assert_eq!(
            parse_inspect_topology_command(&[
                "inspect".to_owned(),
                "topology".to_owned(),
                "--format=runtime-mermaid".to_owned(),
            ])
            .expect("parse")
            .expect("inspect command"),
            InspectTopologyFormat::RuntimeMermaid
        );
        assert_eq!(
            parse_inspect_topology_command(&[
                "inspect".to_owned(),
                "topology".to_owned(),
                "--format=mermaid".to_owned(),
            ])
            .expect("parse")
            .expect("inspect command"),
            InspectTopologyFormat::Mermaid
        );
        assert!(
            parse_inspect_topology_command(&["inspect".to_owned(), "plugins".to_owned()]).is_err()
        );
        assert!(
            parse_inspect_topology_command(&["doctor".to_owned()])
                .expect("parse")
                .is_none()
        );
    }

    #[test]
    fn inspect_topology_builds_snapshot_when_tool_backend_is_invalid() {
        disable_plugins();
        let mut config = AppConfig::default();
        config.modules.search = "missing-search".to_owned();

        let snapshot = build_cli_topology(
            &config,
            None,
            std::path::Path::new("."),
            config.permissions.mode,
        )
        .expect("best-effort topology snapshot");

        assert!(snapshot.slots.iter().any(|slot| slot.id == "search"));
        assert!(snapshot.warnings.iter().any(|warning| {
            warning
                .message
                .contains("inspect could not build search module missing-search")
        }));
        assert!(snapshot.warnings.iter().any(|warning| {
            warning
                .message
                .contains("active module is not registered: search/missing-search")
        }));
    }

    #[test]
    fn app_server_stdio_command_is_exact() {
        assert!(is_app_server_stdio_command(&[
            "server".to_owned(),
            "stdio".to_owned()
        ]));
        assert!(!is_app_server_stdio_command(&["server".to_owned()]));
        assert!(!is_app_server_stdio_command(&[
            "server".to_owned(),
            "stdio".to_owned(),
            "extra".to_owned()
        ]));
    }

    #[test]
    fn app_server_http_command_parses_defaults_and_bind_options() {
        let default_config =
            parse_app_server_http_command(&["server".to_owned(), "http".to_owned()])
                .expect("parse")
                .expect("http command");
        assert_eq!(default_config.bind.to_string(), "127.0.0.1:8787");
        assert!(!default_config.require_session_token);

        let custom_config = parse_app_server_http_command(&[
            "server".to_owned(),
            "http".to_owned(),
            "--host".to_owned(),
            "0.0.0.0".to_owned(),
            "--port".to_owned(),
            "9000".to_owned(),
        ])
        .expect("parse")
        .expect("http command");
        assert_eq!(custom_config.bind.to_string(), "0.0.0.0:9000");

        let token_config = parse_app_server_http_command(&[
            "server".to_owned(),
            "http".to_owned(),
            "--token".to_owned(),
            "secret".to_owned(),
        ])
        .expect("parse")
        .expect("http command");
        assert!(token_config.require_session_token);

        assert!(
            parse_app_server_http_command(&["server".to_owned(), "web".to_owned()])
                .expect("parse")
                .is_none()
        );
        assert!(
            parse_app_server_http_command(&[
                "server".to_owned(),
                "http".to_owned(),
                "--bad".to_owned()
            ])
            .is_err()
        );
    }

    #[test]
    fn doctor_command_is_exact() {
        assert!(is_doctor_command(&["doctor".to_owned()]));
        assert!(!is_doctor_command(&[
            "doctor".to_owned(),
            "extra".to_owned()
        ]));
        assert!(!is_doctor_command(&[
            "tools".to_owned(),
            "doctor".to_owned()
        ]));
    }

    #[test]
    fn eval_report_command_requires_path() {
        assert_eq!(
            parse_eval_report_command(&[
                "eval".to_owned(),
                "report".to_owned(),
                ".proteus/events.jsonl".to_owned()
            ])
            .unwrap(),
            Some(".proteus/events.jsonl")
        );
        assert!(parse_eval_report_command(&["eval".to_owned()]).is_err());
        assert!(parse_eval_report_command(&["eval".to_owned(), "report".to_owned()]).is_err());
        assert_eq!(
            parse_eval_report_command(&["doctor".to_owned()]).unwrap(),
            None
        );
    }

    #[test]
    fn init_command_defaults_to_coding_profile() {
        assert_eq!(
            parse_init_command(&["init".to_owned()]).unwrap(),
            Some(InitProfile::Coding)
        );
        assert_eq!(
            parse_init_command(&["init".to_owned(), "safe".to_owned()]).unwrap(),
            Some(InitProfile::Safe)
        );
        assert_eq!(
            parse_init_command(&["init".to_owned(), "codex".to_owned()]).unwrap(),
            Some(InitProfile::Codex)
        );
        assert!(parse_init_command(&["init".to_owned(), "bad".to_owned()]).is_err());
        assert_eq!(parse_init_command(&["doctor".to_owned()]).unwrap(), None);
    }

    #[test]
    fn init_destination_uses_config_file_or_profile_file_in_dir() {
        assert_eq!(
            init_destination_path(Path::new("/tmp/config.toml"), InitProfile::Coding),
            PathBuf::from("/tmp/config.toml")
        );
        assert_eq!(
            init_destination_path(Path::new("/tmp/configs"), InitProfile::Safe),
            PathBuf::from("/tmp/configs/config.toml")
        );
    }

    #[test]
    fn init_config_path_from_arg_expands_named_config() {
        let expected_codex_path = AppConfig::named_config_destination_path(Path::new("codex"))
            .expect("codex config path");
        assert_eq!(
            init_config_path_from_arg(Path::new("codex")),
            expected_codex_path
        );
        assert_eq!(
            init_config_path_from_arg(Path::new("./codex")),
            PathBuf::from("./codex")
        );
        assert_eq!(
            init_config_path_from_arg(Path::new("codex.config.toml")),
            PathBuf::from("codex.config.toml")
        );
    }

    #[test]
    fn mixed_config_files_warning_lists_sibling_config_files() {
        let dir = tempfile::tempdir().expect("config dir");
        let config = dir.path().join(INIT_CONFIG_FILE);
        std::fs::write(&config, "").expect("config");
        std::fs::write(dir.path().join("00-provider.toml"), "").expect("legacy provider");
        std::fs::write(dir.path().join("10-coding.toml"), "").expect("legacy profile");
        std::fs::write(dir.path().join("notes.md"), "").expect("notes");

        let warning = mixed_config_files_warning(&config).expect("warning");

        assert!(warning.contains("00-provider.toml"));
        assert!(warning.contains("10-coding.toml"));
        assert!(!warning.contains("notes.md"));
        assert!(warning.contains("--config"));
    }

    #[test]
    fn single_config_file_for_warning_resolves_directory_config_toml() {
        let dir = tempfile::tempdir().expect("config dir");
        let config = dir.path().join(INIT_CONFIG_FILE);
        std::fs::write(&config, "").expect("config");

        assert_eq!(
            single_config_file_for_warning(Some(dir.path())),
            Some(config)
        );
    }

    #[test]
    fn doctor_config_root_for_default_config_file_is_config_home() {
        assert_eq!(
            config_root_for_doctor(Some(Path::new("/tmp/agent/configs/config.toml"))),
            Some(PathBuf::from("/tmp/agent"))
        );
    }

    #[tokio::test]
    async fn init_coding_writes_loadable_single_config_file() {
        let dir = tempfile::tempdir().expect("config dir");

        run_init(InitProfile::Coding, Some(dir.path())).expect("init coding");

        let profile = dir.path().join(INIT_CONFIG_FILE);
        assert!(profile.exists());
        let profile_body = std::fs::read_to_string(&profile).expect("profile body");
        assert!(profile_body.starts_with("active_provider = \"anthropic\""));
        assert!(
            !profile_body
                .lines()
                .any(|line| line.trim_start().starts_with("include = "))
        );

        let config = AppConfig::load(Some(dir.path()))
            .await
            .expect("generated config loads");
        let model = config.active_model_config().expect("active model");

        assert_eq!(config.profile.name, "coding-local");
        assert_eq!(config.active_provider.as_deref(), Some("anthropic"));
        assert_eq!(model.provider, "anthropic");
        assert_eq!(config.modules.workflow, "coding.single_loop");
    }

    #[tokio::test]
    async fn init_codex_writes_loadable_single_config_file() {
        let dir = tempfile::tempdir().expect("config dir");

        run_init(InitProfile::Codex, Some(dir.path())).expect("init codex");

        let profile = dir.path().join(INIT_CONFIG_FILE);
        assert!(profile.exists());
        let profile_body = std::fs::read_to_string(&profile).expect("profile body");
        assert!(profile_body.starts_with("active_provider = \"anthropic\""));
        assert!(
            !profile_body
                .lines()
                .any(|line| line.trim_start().starts_with("include = "))
        );

        let config = AppConfig::load(Some(dir.path()))
            .await
            .expect("generated config loads");

        assert_eq!(config.profile.name, "codex-experimental");
        assert_eq!(config.modules.workflow, "coding.codex_loop");
        assert_eq!(config.modules.context, "codex_context");
        assert_eq!(config.modules.compactor, "codex");
        assert_eq!(config.modules.tool_exposure, "codex_dynamic");
    }

    #[test]
    fn doctor_flags_legacy_native_file_tool_handlers() {
        let mut config = AppConfig::default();
        config
            .tools
            .configured
            .push(proteus_core::core::ConfiguredToolConfig {
                name: "read_file".to_owned(),
                description: "old file reader".to_owned(),
                input_schema: serde_json::json!({ "type": "object" }),
                safety: ToolSafety::ReadOnly,
                timeout_ms: None,
                metadata: serde_json::Value::Null,
                executor: ConfiguredToolExecutorConfig::Native {
                    handler: "read_file".to_owned(),
                },
            });

        let mut findings = DoctorFindings::default();
        check_configured_tools(&mut findings, &config);
        assert!(findings.has_errors());
    }

    #[test]
    fn doctor_accepts_fake_model_without_secret() {
        let config = AppConfig::default();
        let catalog = BuiltinModuleCatalog::new();
        let mut findings = DoctorFindings::default();

        check_model_config(&mut findings, &catalog, &config);

        assert!(!findings.has_errors());
        assert!(
            findings
                .entries
                .iter()
                .any(|entry| entry.message == "model secret: not required for fake provider")
        );
    }

    #[test]
    fn doctor_flags_missing_provider_secret_env() {
        const ENV_NAME: &str = "PROTEUS_DOCTOR_TEST_MISSING_API_KEY";
        unsafe {
            std::env::remove_var(ENV_NAME);
        }
        let model = proteus_core::core::ModelConfig {
            provider: "anthropic".to_owned(),
            model: "claude-test".to_owned(),
            stream: false,
            reasoning: proteus_core::domain::ReasoningConfig::default(),
            provider_config: serde_json::json!({ "api_key_env": ENV_NAME }),
        };
        let mut findings = DoctorFindings::default();

        check_model_secret(&mut findings, &model);

        assert!(findings.has_errors());
        assert!(
            findings
                .entries
                .iter()
                .any(|entry| entry.message.contains(ENV_NAME))
        );
    }

    #[test]
    fn doctor_resolves_relative_commands_from_cwd() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("tool.sh"), "#!/bin/sh\n").expect("tool");

        assert!(command_resolves("./tool.sh", dir.path()));
        assert!(!command_resolves("./missing.sh", dir.path()));
    }

    #[test]
    fn doctor_warns_on_short_model_timeout() {
        let mut findings = DoctorFindings::default();

        check_timeout_ms(&mut findings, "runtime.model_timeout_ms", 1_000, 120_000);

        assert!(
            findings
                .entries
                .iter()
                .any(|entry| entry.level == "warn" && entry.message.contains("too low"))
        );
    }

    #[test]
    fn doctor_formats_timeouts_for_readability() {
        assert_eq!(format_timeout_ms(0), "disabled");
        assert_eq!(format_timeout_ms(120_000), "2m");
        assert_eq!(format_timeout_ms(10_800_000), "3h");
        assert_eq!(format_timeout_ms(1_500), "1500ms");
    }

    #[test]
    fn module_list_output_contains_catalog_rows() {
        let manifests = vec![ModuleManifest::builtin(
            "rg",
            ModuleKind::Search,
            &["workspace", "ripgrep"],
        )];
        let rendered = render_module_list(&manifests);

        assert!(rendered.contains("kind"));
        assert!(rendered.contains("search"));
        assert!(rendered.contains("rg"));
        assert!(rendered.contains("workspace,ripgrep"));
    }

    #[test]
    fn tool_list_output_contains_registered_tools() {
        disable_plugins();
        let mut config = AppConfig::default();
        config.modules.patch = "null".to_owned();
        config.tools.path = None;
        // File I/O and shell are plugin-provided; use the remaining builtin
        // tools to exercise render_tool_list without depending on plugins.
        config.tools.enabled = vec!["apply_patch".to_owned(), "search".to_owned()];
        let dir = tempfile::tempdir().expect("temp dir");
        let registry = build_tool_registry_for_listing(&config, dir.path()).unwrap();
        let rendered = render_tool_list(&registry);

        assert!(rendered.contains("name"));
        assert!(rendered.contains("apply_patch"));
        assert!(rendered.contains("builtin:builtin"));
        assert!(rendered.contains("WritesFiles"));
        assert!(rendered.contains("search"));
        assert!(rendered.contains("ReadOnly"));
    }

    #[test]
    fn eval_report_output_contains_core_metrics() {
        let report = proteus_core::core::EvalReport {
            event_log_path: PathBuf::from(".proteus/events.jsonl"),
            events: 9,
            turns_started: 1,
            turns_finished: 1,
            turns_failed: 0,
            model_calls: 2,
            tool_calls: 3,
            tool_failures: 1,
            approvals_requested: 1,
            approvals_resolved: 1,
            approvals_approved: 0,
            approvals_denied: 1,
            estimated_input_tokens: 100,
            provider_input_tokens: 90,
            provider_output_tokens: 30,
            changed_files: vec!["src/lib.rs".to_owned()],
            duration_ms: Some(42),
            failure_reason: None,
        };

        let rendered = render_eval_report(&report);

        assert!(rendered.contains("Status: success"));
        assert!(rendered.contains("Turns: started=1, finished=1, failed=0"));
        assert!(rendered.contains("tool calls: 3 (failures=1)"));
        assert!(rendered.contains("provider_output=30"));
        assert!(rendered.contains("Changed files: src/lib.rs"));
    }
}
