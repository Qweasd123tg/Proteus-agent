use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use async_trait::async_trait;
use clap::{Parser, ValueEnum};
use modular_agent::domain::{AgentOutput, PermissionMode};
use modular_agent::{
    contracts::{ApprovalRequest, ApprovalResponse, ApprovalTransport, EventSink},
    core::{AgentRuntime, AppConfig, BroadcastEventSink, FanoutEventSink, JsonlEventStore},
    modules::ChannelApprovalTransport,
};
use serde_json::Value;
use tokio::time::sleep;

mod tui;

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
    if cli.interactive || cli.task.is_empty() {
        if io::stdin().is_terminal() && io::stdout().is_terminal() {
            let broadcast = Arc::new(BroadcastEventSink::new(1024));
            let jsonl = Arc::new(JsonlEventStore::new(cwd.join(&config.event_log.path)));
            let event_sink: Arc<dyn EventSink> =
                Arc::new(FanoutEventSink::new(vec![jsonl, broadcast.clone()]));
            let (approval_tx, approval_rx) = ChannelApprovalTransport::new(8);
            let runtime = AgentRuntime::builder(config.clone(), cwd.clone())
                .with_config_path(config_path.as_deref())
                .with_event_sink(event_sink)
                .with_approval(Arc::new(approval_tx))
                .build()?;
            return tui::run_tui(runtime, config, cwd, broadcast, approval_rx).await;
        }
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
            return Ok(ApprovalResponse {
                approved: false,
                note: Some(format!(
                    "approval transport is not interactive: {}",
                    request.reason
                )),
            });
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
        Ok(ApprovalResponse {
            approved,
            note: (!approved).then(|| format!("tool call was not approved: {}", request.reason)),
        })
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
