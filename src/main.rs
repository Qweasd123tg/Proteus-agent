use std::{
    io::{self, Write},
    path::PathBuf,
};

use anyhow::Result;
use clap::Parser;
use modular_agent::core::{AgentRuntime, AppConfig};

#[derive(Debug, Parser)]
#[command(author, version, about = "CLI-first modular agent skeleton")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(short, long)]
    interactive: bool,
    #[arg(trailing_var_arg = true)]
    task: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = AppConfig::load(cli.config.as_deref()).await?;
    let cwd = match cli.cwd {
        Some(cwd) => cwd,
        None => std::env::current_dir()?,
    };
    let runtime = AgentRuntime::new(config, cwd)?;

    if cli.interactive || cli.task.is_empty() {
        return run_repl(runtime).await;
    }

    let output = runtime.run(cli.task.join(" ")).await?;
    println!("{}", runtime.render(&output).await?);
    Ok(())
}

async fn run_repl(runtime: AgentRuntime) -> Result<()> {
    println!("Modular Agent REPL");
    println!("Type a task, /help, or /exit.");

    loop {
        print!("agent> ");
        io::stdout().flush()?;

        let mut input = String::new();
        let bytes = io::stdin().read_line(&mut input)?;
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
                runtime.clear_history().await;
                println!("history cleared");
                continue;
            }
            "/history" => {
                println!("history messages: {}", runtime.history_len().await);
                continue;
            }
            "/help" => {
                println!("Commands:");
                println!("  /help            show this help");
                println!("  /history         show in-memory history size");
                println!("  /clear, /reset   clear in-memory history");
                println!("  /exit, /quit     leave the REPL");
                println!("Examples:");
                println!("  read_file Cargo.toml");
                println!("  summarize project");
                continue;
            }
            _ => {}
        }

        match runtime.run(input.to_owned()).await {
            Ok(output) => println!("{}", runtime.render(&output).await?),
            Err(error) => eprintln!("error: {error:#}"),
        }
    }

    Ok(())
}
