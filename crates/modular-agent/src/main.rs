use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use async_trait::async_trait;
use clap::{Parser, ValueEnum};
use modular_agent::app_server::stdio::run_stdio_app_server;
use modular_agent::domain::{AgentOutput, ModuleKind, ModuleManifest, PermissionMode, ToolSafety};
use modular_agent::{
    contracts::{ApprovalRequest, ApprovalResponse, ApprovalTransport},
    core::{AgentRuntime, AppConfig, BuiltinModuleCatalog, ModuleBuildContext},
};
use serde_json::Value;
use tokio::time::sleep;

#[derive(Debug, Parser)]
#[command(author, version, about = "CLI-first modular agent skeleton")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    cwd: Option<PathBuf>,
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
    if is_modules_list_command(&cli.task) {
        let mut catalog = BuiltinModuleCatalog::new();
        if let Some(plugins_dir) = modular_agent::core::default_plugins_dir() {
            let _ = modular_agent::core::load_plugins_from_dir(&plugins_dir, &mut catalog);
        }
        println!("{}", render_module_list(&catalog.manifests()));
        return Ok(());
    }

    let config_path = cli
        .config
        .clone()
        .or_else(AppConfig::default_user_config_path);
    let mut config = AppConfig::load(cli.config.as_deref()).await?;
    config.permissions.mode = resolve_permission_mode(&cli, config.permissions.mode)?;
    let cwd = match cli.cwd {
        Some(cwd) => cwd,
        None => std::env::current_dir()?,
    };
    if is_tools_list_command(&cli.task) {
        let registry = build_tool_registry_for_listing(&config, &cwd)?;
        println!("{}", render_tool_list(&registry));
        return Ok(());
    }
    if is_app_server_stdio_command(&cli.task) {
        return run_stdio_app_server(config, cwd, config_path).await;
    }
    if cli.interactive || cli.task.is_empty() {
        let runtime = AgentRuntime::new_with_config_path_and_approval_transport(
            config.clone(),
            cwd.clone(),
            config_path.as_deref(),
            terminal_approval_transport(),
        )?;
        return run_repl(runtime, config, cwd).await;
    }

    let runtime = AgentRuntime::new_with_config_path_and_approval_transport(
        config.clone(),
        cwd.clone(),
        config_path.as_deref(),
        terminal_approval_transport(),
    )?;
    let output = runtime.run(cli.task.join(" ")).await?;
    println!("{}", runtime.render(&output).await?);
    Ok(())
}

fn is_modules_list_command(task: &[String]) -> bool {
    matches!(task, [module, command] if module == "modules" && command == "list")
}

fn is_tools_list_command(task: &[String]) -> bool {
    matches!(task, [tool, command] if tool == "tools" && command == "list")
}

fn is_app_server_stdio_command(task: &[String]) -> bool {
    matches!(task, [server, transport] if server == "server" && transport == "stdio")
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

fn build_tool_registry_for_listing(
    config: &AppConfig,
    cwd: &std::path::Path,
) -> Result<modular_agent::contracts::ToolRegistry> {
    let catalog = BuiltinModuleCatalog::new();
    let build_ctx = ModuleBuildContext { config, cwd };
    let search = catalog.build_search(&config.modules.search, &build_ctx)?;
    let patch = catalog.build_patch(&config.modules.patch, &build_ctx)?;
    catalog.build_tools(&build_ctx, search, patch)
}

fn render_tool_list(registry: &modular_agent::contracts::ToolRegistry) -> String {
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
                            "/exit, /quit     leave the REPL".to_owned(),
                            "examples: read_file Cargo.toml | summarize project".to_owned(),
                        ],
                    )
                );
                continue;
            }
            _ => {}
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
        "Modular Agent REPL".to_owned(),
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
    lines.push(format!(
        "status: components={} position={} frame={}",
        config.renderer.statusline.components.join(","),
        config.renderer.statusline.position,
        config.renderer.statusline.frame
    ));

    Ok(small_block("Modular Agent", &lines))
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
    let context = footer_context(config, output);
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

fn footer_context(config: &AppConfig, output: &AgentOutput) -> String {
    let context = output.metadata.get("context");
    let tokens = context
        .and_then(|context| context.get("token_estimate"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let chunks = context
        .and_then(|context| context.get("chunks"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let max_tokens = config
        .renderer
        .statusline
        .context
        .max_tokens
        .unwrap_or(200_000)
        .max(1);
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
    use modular_agent::domain::ModuleManifest;

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
        let mut config = AppConfig::default();
        config.tools.path = None;
        config.tools.enabled = vec!["read_file".to_owned(), "shell".to_owned()];
        let dir = tempfile::tempdir().expect("temp dir");
        let registry = build_tool_registry_for_listing(&config, dir.path()).unwrap();
        let rendered = render_tool_list(&registry);

        assert!(rendered.contains("name"));
        assert!(rendered.contains("read_file"));
        assert!(rendered.contains("builtin:builtin"));
        assert!(rendered.contains("ReadOnly"));
        assert!(rendered.contains("shell"));
        assert!(rendered.contains("RunsCommands"));
    }
}
