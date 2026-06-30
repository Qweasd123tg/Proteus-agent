use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use async_trait::async_trait;
use clap::{Parser, ValueEnum};
use proteus_core::app_server::{http::run_http_app_server, stdio::run_stdio_app_server};
use proteus_core::domain::{
    AgentOutput, ModuleKind, ModuleManifest, PermissionMode, ToolSafety, new_thread_id,
};
use proteus_core::{
    contracts::{ApprovalRequest, ApprovalResponse, ApprovalTransport},
    core::{
        AgentRuntime, AppConfig, BuiltinModuleCatalog, ModuleBuildContext, ModuleEpoch,
        TopologyBuildInput, TopologyWarning, build_topology_snapshot, normalize_session_dir_path,
        render_topology_map, render_topology_markdown, render_topology_mermaid,
        render_topology_runtime_mermaid, render_topology_runtime_path, render_topology_table,
        session_id_from_session_dir,
    },
};
use serde_json::Value;
use tokio::time::sleep;

mod cli_commands;
mod cli_doctor;
mod cli_init;

use cli_commands::{
    InspectTopologyFormat, is_app_server_stdio_command, is_doctor_command, is_modules_list_command,
    is_tools_list_command, parse_app_server_http_command, parse_eval_report_command,
    parse_inspect_topology_command,
};
use cli_doctor::run_doctor;
use cli_init::{parse_init_command, run_init};

#[cfg(test)]
use cli_doctor::{
    DoctorFindings, check_configured_tools, check_model_config, check_model_secret,
    check_timeout_ms, command_resolves, config_root_for_doctor, format_timeout_ms,
};
#[cfg(test)]
use cli_init::{
    INIT_CONFIG_FILE, InitProfile, init_config_path_from_arg, init_destination_path,
    mixed_config_files_warning, single_config_file_for_warning,
};
#[cfg(test)]
use proteus_core::core::ConfiguredToolExecutorConfig;
#[cfg(test)]
use std::path::Path;

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
#[path = "main_tests.rs"]
mod tests;
