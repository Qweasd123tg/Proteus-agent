//! Terminal UI клиент для modular-agent.
//!
//! Запускает ядро как subprocess через `agent server stdio`, читает поток
//! `AppServerEvent`, шлёт user input как `StdioRequest::Send`. Визуал на
//! ratatui/crossterm. Клиент depend только на `agent-contracts`, не на
//! самом ядре — границa client/core проведена через wire protocol.

mod app_loop;
mod bottom_pane;
mod cards;
mod commands;
mod driver;
mod history_insert;
mod inline_terminal;
mod input;
mod markdown;
mod profiles;
mod session_picker;
mod slash_commands;
mod state;
mod terminal_host;
mod terminal_surface;
mod transcript;
mod visual;

use anyhow::Result;

use crate::{
    app_loop::run_app,
    profiles::{apply_profile, parse_args},
    terminal_host::{enter_terminal, install_panic_hook, leave_terminal},
};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = apply_profile(parse_args(&args)?)?;

    // Перехват panic'а: если TUI упадёт — восстанавливаем терминал в
    // нормальный режим и пишем stack trace в файл, чтобы ты мог его
    // увидеть после выхода.
    install_panic_hook();

    let mut terminal = enter_terminal()?;
    let result = run_app(&mut terminal, cfg).await;
    leave_terminal(&mut terminal)?;
    if let Err(ref err) = result {
        eprintln!("agent-tui: {err:#}");
    }
    result
}
