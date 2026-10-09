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
use tokio::time::sleep;

mod cli_app;
mod cli_commands;
mod cli_doctor;
mod cli_init;
mod cli_prompt_replay;
mod cli_workflow_replay;

use cli_app::CliAppClient;
use cli_commands::{CliCommand, InspectPlanFormat, InspectTopologyFormat, parse_cli_command};
#[cfg(test)]
use cli_commands::{
    parse_app_server_http_command, parse_eval_report_command, parse_inspect_plan_command,
    parse_inspect_topology_command, parse_prompt_replay_command, parse_workflow_replay_command,
};
use cli_doctor::run_doctor;
use cli_init::run_init;
use cli_prompt_replay::run_prompt_replay;
use cli_workflow_replay::run_workflow_replay;

#[cfg(test)]
use cli_doctor::{
    DoctorFindings, check_external_commands, check_model_config,
    check_module_config_tool_references, check_timeout_ms, command_resolves, format_timeout_ms,
};
#[cfg(test)]
use cli_init::{
    INIT_CONFIG_FILE, InitProfile, init_config_path_from_arg, init_destination_path,
    mixed_config_files_warning, parse_init_command, single_config_file_for_warning,
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
    let command = parse_cli_command(&cli.task)?;
    if let CliCommand::Init(profile) = command {
        return run_init(profile, cli.config.as_deref());
    }
    if matches!(command, CliCommand::ModulesList) {
        let config = AppConfig::load(cli.config.as_deref()).await?;
        let catalog = proteus_core::core::ModuleCatalog::from_config(&config)?;
        println!("{}", render_module_list(&catalog.manifests()));
        return Ok(());
    }
    if let CliCommand::EvalReport(path) = command {
        let report = proteus_core::core::read_eval_report(path)?;
        println!("{}", render_eval_report(&report));
        return Ok(());
    }
    let config_path = AppConfig::resolve_config_path(cli.config.as_deref()).await?;
    let cwd = match cli.cwd {
        Some(ref cwd) => cwd.clone(),
        None => std::env::current_dir()?,
    };
    if let CliCommand::Doctor(scope) = command {
        return run_doctor(cli.config.as_deref(), config_path.as_deref(), &cwd, scope).await;
    }

    let mut config = AppConfig::load(cli.config.as_deref()).await?;
    if let CliCommand::PromptReplay(command) = command {
        println!("{}", run_prompt_replay(&config, &cwd, command).await?);
        return Ok(());
    }
    if let CliCommand::WorkflowReplay(command) = command {
        println!("{}", run_workflow_replay(&config, command).await?);
        return Ok(());
    }
    config.permissions.mode = resolve_permission_mode(&cli, config.permissions.mode)?;
    if cli.new_session && cli.resume_session.is_some() {
        anyhow::bail!("--new-session conflicts with --resume-session");
    }
    if let CliCommand::InspectPlan(format) = command {
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
    if let CliCommand::InspectTopology(format) = command {
        let snapshot = build_cli_topology(
            &config,
            config_path.as_deref(),
            &cwd,
            config.permissions.mode,
        )?;
        println!("{}", render_inspect_topology(&snapshot, format)?);
        return Ok(());
    }
    if matches!(command, CliCommand::ToolsList) {
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
    if matches!(command, CliCommand::ServerAcp) {
        if cli.resume_session.is_some() || cli.new_session || cli.interactive {
            bail!(
                "ACP sessions are managed by the client; omit --resume-session, --new-session and --interactive"
            );
        }
        return proteus_core::app_server::acp::run_acp_server(config, config_path).await;
    }
    if matches!(command, CliCommand::ServerStdio) {
        return run_stdio_app_server(
            config,
            cwd,
            config_path,
            cli.resume_session,
            cli.new_session,
        )
        .await;
    }
    if let CliCommand::ServerHttp(http_config) = command {
        return run_http_app_server(
            config,
            cwd,
            config_path,
            cli.resume_session,
            cli.new_session,
            http_config,
        )
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
    let mut tools = catalog.build_tools_for_inspection(config, cwd)?;
    agent_control.register_tools(&mut tools, config.runtime.workflow_timeout_ms)?;
    if let Some(model_config) = config.selected_model_config()? {
        let model = catalog.build_model_adapter(&model_config, cwd)?;
        register_provider_hosted_tools(
            &mut tools,
            model.id().as_ref(),
            model.provider_hosted_tools(&model_config.model_ref())?,
        )?;
    }
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
    let hosted_tools = config.selected_model_config().and_then(|model_config| {
        model_config
            .map(|model_config| {
                let model = catalog.build_model_adapter(&model_config, cwd)?;
                Ok((
                    model.id().into_owned(),
                    model.provider_hosted_tools(&model_config.model_ref())?,
                ))
            })
            .transpose()
    });
    let hosted_tools = match hosted_tools {
        Ok(hosted) => hosted,
        Err(error) => {
            extra_warnings.push(TopologyWarning::error(format!(
                "inspect could not build model-hosted tools: {error:#}"
            )));
            None
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
            if let Some((source, specs)) = hosted_tools {
                if let Err(error) = register_provider_hosted_tools(&mut tools, &source, specs) {
                    extra_warnings.push(TopologyWarning::error(format!(
                        "inspect could not register model-hosted tools: {error:#}"
                    )));
                }
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

async fn run_repl(client: &mut CliAppClient) -> Result<()> {
    let mut config = client.config_summary().await?;
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

        if matches!(input, "/exit" | "/quit") {
            break;
        }
        let prompt = if input.starts_with('/') && !input.starts_with("//") {
            match client.execute_command(input.to_owned()).await {
                Ok(proteus_contracts::app_protocol::commands::CommandOutput::Display { text }) => {
                    println!("{text}");
                    config = client.config_summary().await?;
                    footer = initial_footer(&config)?;
                    continue;
                }
                Ok(proteus_contracts::app_protocol::commands::CommandOutput::Prompt { text }) => {
                    text
                }
                Err(error) => {
                    eprintln!("error: {error:#}");
                    continue;
                }
            }
        } else {
            input
                .strip_prefix("//")
                .map(|rest| format!("/{rest}"))
                .unwrap_or_else(|| input.to_owned())
        };
        match run_with_spinner(client, prompt, tty_composer).await {
            Ok(output) => {
                print_assistant_output(&output.text, tty_composer).await?;
                footer = footer_from_output(&config, &output)?;
            }
            Err(error) => eprintln!("error: {error:#}"),
        }
    }

    Ok(())
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

#[path = "main/repl_render.rs"]
mod repl_render;
use repl_render::{footer_from_output, initial_footer, repl_header};

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/support/model.rs"]
mod test_model;
