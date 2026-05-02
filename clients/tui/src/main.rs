//! Terminal UI клиент для modular-agent.
//!
//! Запускает ядро как subprocess через `agent server stdio`, читает поток
//! `AppServerEvent`, шлёт user input как `StdioRequest::Send`. Визуал на
//! ratatui/crossterm. Клиент depend только на `agent-contracts`, не на
//! самом ядре — границa client/core проведена через wire protocol.

mod driver;
mod markdown;
mod state;
mod visual;

use std::{io, path::PathBuf, time::Duration};

use agent_contracts::app_protocol::{StdioOutput, StdioRequest};
use std::fmt;

use anyhow::{Context, Result};
use crossterm::{
    Command,
    event::{self, Event as CTerm, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    driver::{AgentDriver, DriverConfig},
    state::AppState,
    visual::VisualSurface,
};

/// DECSET 1007 — alternate scroll mode. Терминал сам переводит wheel
/// в клавиши Up/Down. Выделение текста мышью остаётся стандартным,
/// потому что мы НЕ включаем EnableMouseCapture. Подсмотрено в
/// OpenAI codex-rs/tui/src/tui.rs.
#[derive(Debug, Clone, Copy)]
struct EnableAlternateScroll;

impl Command for EnableAlternateScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[?1007h")
    }
    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        Err(std::io::Error::other("use ANSI instead"))
    }
    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy)]
struct DisableAlternateScroll;

impl Command for DisableAlternateScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[?1007l")
    }
    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        Err(std::io::Error::other("use ANSI instead"))
    }
    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = parse_args(&args)?;

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

fn install_panic_hook() {
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        );

        let backtrace = std::backtrace::Backtrace::force_capture();
        let msg = format!("=== TUI panic ===\n{info}\n\nbacktrace:\n{backtrace}\n",);

        eprintln!("{msg}");

        let path = std::env::temp_dir().join("agent-tui-panic.log");
        let _ = std::fs::write(&path, &msg);
        eprintln!("panic log: {}", path.display());
    }));
}

struct Cli {
    agent_bin: Option<PathBuf>,
    config_path: Option<PathBuf>,
    cwd: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Cli> {
    let mut agent_bin = None;
    let mut config_path = None;
    let mut cwd = None;
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--agent-bin" => {
                agent_bin = iter
                    .next()
                    .map(PathBuf::from)
                    .context("--agent-bin requires value")
                    .ok();
            }
            "--config" => {
                config_path = iter
                    .next()
                    .map(PathBuf::from)
                    .context("--config requires value")
                    .ok();
            }
            "--cwd" => {
                cwd = iter
                    .next()
                    .map(PathBuf::from)
                    .context("--cwd requires value")
                    .ok();
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown arg: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }
    Ok(Cli {
        agent_bin,
        config_path,
        cwd,
    })
}

fn print_help() {
    eprintln!(
        "agent-tui — terminal UI for modular-agent\n\
         \n\
         usage:\n\
           agent-tui [--agent-bin PATH] [--config PATH] [--cwd PATH]\n\
         \n\
         options:\n\
           --agent-bin PATH    path to the modular-agent binary (default: in $PATH)\n\
           --config PATH       path to agent config (toml or json)\n\
           --cwd PATH          workspace directory for the agent\n\
           --help, -h          show this help"
    );
}

fn enter_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    // EnterAlternateScreen + alternate scroll mode.
    //
    // EnableMouseCapture НЕ включаем — с ним терминал отдаёт все мышиные
    // события приложению, и штатное выделение текста + копирование через
    // ОС ломаются. Вместо этого используется alternate scroll (DECSET 1007):
    // терминал сам транслирует колёсико в Up/Down arrows, которые мы ловим
    // как обычные KeyCode::Up/Down. Выделение мышью при этом работает
    // ровно как в bash. Паттерн подсмотрен у OpenAI Codex CLI.
    execute!(out, EnterAlternateScreen, EnableAlternateScroll)?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

fn leave_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableAlternateScroll,
        LeaveAlternateScreen,
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, cli: Cli) -> Result<()> {
    let cwd = cli
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mut driver = AgentDriver::spawn(DriverConfig {
        agent_bin: cli.agent_bin,
        config_path: cli.config_path.clone(),
        cwd: Some(cwd.clone()),
    })
    .await?;

    let mut state = AppState::new(cwd, cli.config_path);
    let surface = VisualSurface::default();

    // Crossterm входные события читаем в отдельном blocking thread и
    // переправляем через mpsc. Из async loop работать с
    // crossterm::event::read напрямую нельзя — это блокирует tokio worker.
    let (input_tx, mut input_rx) = tokio::sync::mpsc::channel::<CTerm>(64);
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if input_tx.blocking_send(ev).is_err() {
                break;
            }
        }
    });

    let mut tick = tokio::time::interval(Duration::from_millis(120));
    let mut dirty = true;

    loop {
        if dirty {
            terminal.draw(|frame| surface.render(frame, &state.visual_state()))?;
            dirty = false;
        }

        if state.should_quit {
            break;
        }

        tokio::select! {
            biased;

            output = driver.events.recv() => {
                match output {
                    Some(StdioOutput::Event { event }) => {
                        state.ingest(*event);
                        dirty = true;
                    }
                    Some(StdioOutput::Response { ok, error, .. }) => {
                        if !ok {
                            state.push_error(error.unwrap_or_else(|| "request failed".into()));
                            dirty = true;
                        }
                    }
                    Some(_) => {}
                    None => {
                        state.push_error("agent process exited unexpectedly".into());
                        dirty = true;
                        state.should_quit = true;
                    }
                }
            }

            term_event = input_rx.recv() => {
                match term_event {
                    Some(ev) => {
                        if handle_term_event(&mut state, &mut driver, ev).await? {
                            dirty = true;
                        }
                    }
                    None => {
                        // Input thread завершился — редкий случай, продолжаем.
                    }
                }
            }

            _ = tick.tick() => {
                if state.advance_spinner() {
                    dirty = true;
                }
            }
        }
    }

    let _ = driver.shutdown().await;
    Ok(())
}

async fn handle_term_event(
    state: &mut AppState,
    driver: &mut AgentDriver,
    ev: CTerm,
) -> Result<bool> {
    match ev {
        CTerm::Key(key) if key.kind == KeyEventKind::Press => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('c') => {
                        state.should_quit = true;
                        return Ok(true);
                    }
                    KeyCode::Char('l') => {
                        // Очистка истории — отправляем ClearHistory.
                        driver
                            .send(&StdioRequest::ClearHistory { id: None })
                            .await?;
                        state.clear_transcript();
                        return Ok(true);
                    }
                    _ => {}
                }
            }

            // Если показан approval — обрабатываем y/n.
            if state.has_pending_approval() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        if let Some(id) = state.take_pending_approval_id() {
                            driver
                                .send(&StdioRequest::Approval {
                                    id: None,
                                    approval_id: id,
                                    approved: true,
                                    note: None,
                                    cache: agent_contracts::contracts::ApprovalCacheScope::None,
                                })
                                .await?;
                            return Ok(true);
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        if let Some(id) = state.take_pending_approval_id() {
                            driver
                                .send(&StdioRequest::Approval {
                                    id: None,
                                    approval_id: id,
                                    approved: false,
                                    note: Some("denied by user".into()),
                                    cache: agent_contracts::contracts::ApprovalCacheScope::None,
                                })
                                .await?;
                            return Ok(true);
                        }
                    }
                    _ => {}
                }
                return Ok(false);
            }

            match key.code {
                KeyCode::Enter => {
                    if let Some(text) = state.take_input_for_send() {
                        driver
                            .send(&StdioRequest::Send {
                                id: None,
                                text: text.clone(),
                            })
                            .await?;
                        state.mark_user_sent(text);
                        return Ok(true);
                    }
                }
                KeyCode::Backspace => {
                    state.backspace();
                    return Ok(true);
                }
                KeyCode::Char(ch) => {
                    state.type_char(ch);
                    return Ok(true);
                }
                KeyCode::PageUp => {
                    state.scroll_up(5);
                    return Ok(true);
                }
                KeyCode::PageDown => {
                    state.scroll_down(5);
                    return Ok(true);
                }
                // Wheel scroll через alternate-scroll mode приходит как
                // Up/Down arrows. Ловим и скроллим транскрипт на 1 строку.
                KeyCode::Up => {
                    state.scroll_up(1);
                    return Ok(true);
                }
                KeyCode::Down => {
                    state.scroll_down(1);
                    return Ok(true);
                }
                KeyCode::End => {
                    state.scroll_to_bottom();
                    return Ok(true);
                }
                KeyCode::Esc => {
                    // Esc на пустом состоянии — отмена последнего turn'а.
                    if state.pending_model {
                        // TODO: cancel. Нужен target_id последнего Send.
                    }
                }
                _ => {}
            }
        }
        CTerm::Resize(_, _) => return Ok(true),
        _ => {}
    }
    Ok(false)
}
