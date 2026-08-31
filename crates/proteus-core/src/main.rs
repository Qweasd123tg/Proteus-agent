use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};
use proteus_contracts::{
    contracts::ToolRegistry,
    domain::{AgentOutput, ModuleManifest, PermissionMode, ToolSafety},
};
use proteus_core::app_server::{http::run_http_app_server, stdio::run_stdio_app_server};
use proteus_core::core::{
    AgentControlRuntime, AppConfig, AssemblyPlan, ModuleCatalog, ModuleEpoch, TopologyBuildInput,
    TopologySnapshot, TopologyWarning, build_topology_snapshot, register_provider_hosted_tools,
    render_assembly_plan, render_topology_map, render_topology_markdown, render_topology_mermaid,
    render_topology_runtime_mermaid, render_topology_runtime_path, render_topology_table,
};
use serde_json::Value;
use tokio::time::sleep;

mod cli_app;
mod cli_commands;
mod cli_doctor;
mod cli_init;
mod cli_prompt_replay;
mod cli_workflow_replay;

use cli_app::CliAppClient;
use cli_commands::{
    InspectPlanFormat, InspectTopologyFormat, is_app_server_stdio_command, is_doctor_command,
    is_modules_list_command, is_tools_list_command, parse_app_server_http_command,
    parse_eval_report_command, parse_inspect_plan_command, parse_inspect_topology_command,
    parse_prompt_replay_command, parse_workflow_replay_command,
};
use cli_doctor::run_doctor;
use cli_init::{parse_init_command, run_init};
use cli_prompt_replay::run_prompt_replay;
use cli_workflow_replay::run_workflow_replay;

#[cfg(test)]
use cli_doctor::{
    DoctorFindings, check_external_commands, check_model_config, check_model_secret,
    check_module_config_tool_references, check_timeout_ms, command_resolves, format_timeout_ms,
};
#[cfg(test)]
use cli_init::{
    INIT_CONFIG_FILE, InitProfile, init_config_path_from_arg, init_destination_path,
    mixed_config_files_warning, single_config_file_for_warning,
};
#[cfg(test)]
use std::path::Path;

#[derive(Debug, Parser)]
#[command(
    name = "proteus",
    author,
    version,
    about = "CLI-first Proteus skeleton"
)]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long)]
    resume_session: Option<PathBuf>,
    /// Всегда стартовать свежую session вместо resume последней workspace
    /// session (используется subagent process runner-ом для детей).
    #[arg(long)]
    new_session: bool,
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
        let config = AppConfig::load(cli.config.as_deref()).await?;
        let catalog = proteus_core::core::ModuleCatalog::from_config(&config)?;
        println!("{}", render_module_list(&catalog.manifests()));
        return Ok(());
    }
    if let Some(path) = parse_eval_report_command(&cli.task)? {
        let report = proteus_core::core::read_eval_report(path)?;
        println!("{}", render_eval_report(&report));
        return Ok(());
    }
    let prompt_replay = parse_prompt_replay_command(&cli.task)?;
    let workflow_replay = parse_workflow_replay_command(&cli.task)?;

    let config_path = AppConfig::resolve_config_path(cli.config.as_deref()).await?;
    let cwd = match cli.cwd {
        Some(ref cwd) => cwd.clone(),
        None => std::env::current_dir()?,
    };
    if is_doctor_command(&cli.task) {
        return run_doctor(cli.config.as_deref(), config_path.as_deref(), &cwd).await;
    }

    let mut config = AppConfig::load(cli.config.as_deref()).await?;
    if let Some(command) = prompt_replay {
        println!("{}", run_prompt_replay(&config, command).await?);
        return Ok(());
    }
    if let Some(command) = workflow_replay {
        println!("{}", run_workflow_replay(&config, command).await?);
        return Ok(());
    }
    config.permissions.mode = resolve_permission_mode(&cli, config.permissions.mode)?;
    if cli.new_session && cli.resume_session.is_some() {
        anyhow::bail!("--new-session conflicts with --resume-session");
    }
    if let Some(format) = parse_inspect_plan_command(&cli.task)? {
        let (plan, _) = resolve_cli_assembly(
            &config,
            config_path.as_deref(),
            &cwd,
            config.permissions.mode,
        )?;
        println!("{}", render_inspect_plan(&plan, format)?);
        plan.ensure_valid()?;
        return Ok(());
    }
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
        let (plan, catalog) = resolve_cli_assembly(
            &config,
            config_path.as_deref(),
            &cwd,
            config.permissions.mode,
        )?;
        plan.ensure_valid()?;
        let registry = build_tool_registry_for_listing(&plan, &catalog)?;
        println!("{}", render_tool_list(&registry));
        return Ok(());
    }
    if is_app_server_stdio_command(&cli.task) {
        return run_stdio_app_server(
            config,
            cwd,
            config_path,
            cli.resume_session,
            cli.new_session,
        )
        .await;
    }
    if let Some(http_config) = parse_app_server_http_command(&cli.task)? {
        return run_http_app_server(config, cwd, config_path, cli.resume_session, http_config)
            .await;
    }
    if cli.interactive || cli.task.is_empty() {
        let mut client = CliAppClient::launch(
            config_path.as_deref(),
            &cwd,
            cli.resume_session.as_deref(),
            config.permissions.mode,
        )
        .await?;
        let result = run_repl(&mut client).await;
        let shutdown = client.shutdown().await;
        result?;
        return shutdown;
    }

    let mut client = CliAppClient::launch(
        config_path.as_deref(),
        &cwd,
        cli.resume_session.as_deref(),
        config.permissions.mode,
    )
    .await?;
    let output = client.send(cli.task.join(" ")).await;
    let shutdown = client.shutdown().await;
    let output = output?;
    shutdown?;
    println!("{}", output.text);
    Ok(())
}

fn render_module_list(manifests: &[ModuleManifest]) -> String {
    let rows = manifests
        .iter()
        .map(|manifest| {
            [
                manifest.kind.as_str().to_owned(),
                manifest.id.clone(),
                manifest.capabilities.join(","),
                manifest.description.clone().unwrap_or_default(),
            ]
        })
        .collect::<Vec<_>>();

    render_table(["kind", "id", "capabilities", "description"], &rows)
}

fn build_tool_registry_for_listing(
    plan: &AssemblyPlan,
    catalog: &ModuleCatalog,
) -> Result<ToolRegistry> {
    let config = plan.config();
    let cwd = plan.cwd();
    let agent_control = AgentControlRuntime::from_config(&config.agent_control)?;
    let model_config = plan.model_config()?;
    let model = catalog.build_model_adapter(&model_config)?;
    let mut tools = catalog.build_tools_for_inspection(config, cwd)?;
    agent_control.register_tools(&mut tools, config.runtime.workflow_timeout_ms)?;
    register_provider_hosted_tools(
        &mut tools,
        model.id().as_ref(),
        model.provider_hosted_tools(&model_config.model_ref()),
    )?;
    Ok(tools)
}

fn build_cli_topology(
    config: &AppConfig,
    config_path: Option<&std::path::Path>,
    cwd: &std::path::Path,
    permission_mode: PermissionMode,
) -> Result<TopologySnapshot> {
    let (plan, catalog) = resolve_cli_assembly(config, config_path, cwd, permission_mode)?;
    let config = plan.config();
    let mut extra_warnings = Vec::new();
    let agent_control = match AgentControlRuntime::from_config(&config.agent_control) {
        Ok(control) => control,
        Err(error) => {
            extra_warnings.push(TopologyWarning::error(format!(
                "inspect could not build agent control: {error:#}"
            )));
            AgentControlRuntime::disabled()
        }
    };
    let hosted_tools = config.active_model_config().and_then(|model_config| {
        let model = catalog.build_model_adapter(&model_config)?;
        Ok((
            model.id().into_owned(),
            model.provider_hosted_tools(&model_config.model_ref()),
        ))
    });
    let (hosted_source, hosted_specs) = match hosted_tools {
        Ok(hosted) => hosted,
        Err(error) => {
            extra_warnings.push(TopologyWarning::error(format!(
                "inspect could not build model-hosted tools: {error:#}"
            )));
            ("unavailable-model".to_owned(), Vec::new())
        }
    };
    let tool_entries = match catalog.build_tools_for_inspection(config, cwd) {
        Ok(mut tools) => {
            if let Err(error) =
                agent_control.register_tools(&mut tools, config.runtime.workflow_timeout_ms)
            {
                extra_warnings.push(TopologyWarning::error(format!(
                    "inspect could not register agent-control tools: {error:#}"
                )));
            }
            if let Err(error) =
                register_provider_hosted_tools(&mut tools, &hosted_source, hosted_specs)
            {
                extra_warnings.push(TopologyWarning::error(format!(
                    "inspect could not register model-hosted tools: {error:#}"
                )));
            }
            tools.entries()
        }
        Err(error) => {
            extra_warnings.push(TopologyWarning::error(format!(
                "inspect could not build ToolRegistry: {error:#}"
            )));
            Vec::new()
        }
    };

    Ok(build_topology_snapshot(TopologyBuildInput {
        plan: &plan,
        tools: &tool_entries,
        module_epoch: ModuleEpoch::initial(),
        permission_mode,
        extra_warnings,
    }))
}

fn resolve_cli_assembly(
    config: &AppConfig,
    config_path: Option<&std::path::Path>,
    cwd: &std::path::Path,
    permission_mode: PermissionMode,
) -> Result<(AssemblyPlan, ModuleCatalog)> {
    let mut resolved_config = config.clone();
    resolved_config.permissions.mode = permission_mode;
    let catalog = ModuleCatalog::from_config(&resolved_config)?;
    let plan = AssemblyPlan::resolve(resolved_config, config_path, cwd.to_path_buf(), &catalog)?;
    Ok((plan, catalog))
}

fn render_inspect_plan(plan: &AssemblyPlan, format: InspectPlanFormat) -> Result<String> {
    match format {
        InspectPlanFormat::Text => Ok(render_assembly_plan(plan)),
        InspectPlanFormat::Json => serde_json::to_string_pretty(plan).map_err(Into::into),
    }
}

fn render_inspect_topology(
    snapshot: &TopologySnapshot,
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

fn render_tool_list(registry: &ToolRegistry) -> String {
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
    lines.push(format!("Eval report: {}", report.journal_path.display()));
    lines.push(format!(
        "Status: {}",
        if report.succeeded() {
            "success"
        } else {
            "failed"
        }
    ));
    lines.push(format!("Journal records: {}", report.records));
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

/// Реализует slash-команду `/remember KIND TEXT` в REPL.
///
/// Парсинг: первое слово — `kind` (`preference` или `fact`). Остальное —
/// `content`. Если первое слово не валидный kind, всё идёт как `fact`
/// content. Это удобный shortcut: `/remember project uses pnpm` просто
/// работает как fact.
async fn handle_remember(client: &mut CliAppClient, rest: &str) -> Result<String> {
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
    let result = client.remember(kind, content).await?;
    Ok(format!("stored ({}): {}", result.kind, result.content))
}

async fn run_repl(client: &mut CliAppClient) -> Result<()> {
    let config = client.config_summary().await?;
    println!("{}", repl_header(&config)?);
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
                client.clear_history().await?;
                println!("{}", small_block("state", &["history cleared".to_owned()]));
                continue;
            }
            "/history" => {
                let history = client.history_summary().await?;
                println!(
                    "{}",
                    small_block("history", &[format!("messages: {}", history.messages)])
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
            match handle_remember(client, rest).await {
                Ok(message) => {
                    println!("{}", small_block("memory", &[message]));
                }
                Err(error) => {
                    eprintln!("error: {error:#}");
                }
            }
            continue;
        }

        match run_with_spinner(client, input.to_owned(), tty_composer).await {
            Ok(output) => {
                print_assistant_output(&output.text, tty_composer).await?;
                footer = footer_from_output(&config, &output)?;
            }
            Err(error) => eprintln!("error: {error:#}"),
        }
    }

    Ok(())
}

fn repl_header(config: &Value) -> Result<String> {
    let profile = config_string(config, &["profile"])?;
    let model = config_string(config, &["model", "label"])?;
    let cwd = config_string(config, &["cwd"])?;
    let modules = config
        .get("modules")
        .and_then(Value::as_array)
        .map(|modules| {
            modules
                .iter()
                .filter_map(|module| {
                    Some(format!(
                        "{}={}",
                        module.get("slot")?.as_str()?,
                        module.get("id")?.as_str()?
                    ))
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let tools = config
        .get("tools_enabled")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let mut lines = vec![
        "Proteus REPL".to_owned(),
        "type a task, /help, or /exit".to_owned(),
        format!("profile: {profile}"),
        format!("model: {model}"),
        format!("cwd: {cwd}"),
        format!("modules: {modules}"),
        format!("tools: {tools}"),
    ];
    if let Some(session_dir) = config.get("session_dir").and_then(Value::as_str) {
        lines.push(format!("session: {session_dir}"));
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
    client: &mut CliAppClient,
    input: String,
    tty_composer: bool,
) -> Result<AgentOutput> {
    let run = client.send(input);
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

fn initial_footer(config: &Value) -> Result<String> {
    let model = config_string(config, &["model", "label"])?;
    Ok(format!(
        "? for shortcuts    model {model} · Context waiting"
    ))
}

fn footer_from_output(config: &Value, output: &AgentOutput) -> Result<String> {
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

fn footer_model(config: &Value, output: &AgentOutput) -> Result<String> {
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

    Ok(format!(
        "model {}",
        config_string(config, &["model", "label"])?
    ))
}

fn config_string<'a>(config: &'a Value, path: &[&str]) -> Result<&'a str> {
    let mut value = config;
    for segment in path {
        value = value
            .get(*segment)
            .ok_or_else(|| anyhow::anyhow!("app-server config is missing {segment}"))?;
    }
    value.as_str().ok_or_else(|| {
        anyhow::anyhow!("app-server config field {} is not a string", path.join("."))
    })
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
