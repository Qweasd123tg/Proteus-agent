//! Terminal UI клиент для modular-agent.
//!
//! Запускает ядро как subprocess через `agent server stdio`, читает поток
//! `AppServerEvent`, шлёт user input как `StdioRequest::Send`. Визуал на
//! ratatui/crossterm. Клиент depend только на `agent-contracts`, не на
//! самом ядре — границa client/core проведена через wire protocol.

mod driver;
mod state;
mod visual;

use std::{io, path::PathBuf, time::Duration};

use agent_contracts::app_protocol::{StdioOutput, StdioRequest};
use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event as CTerm, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::time::Instant;

use crate::{
    driver::{AgentDriver, DriverConfig},
    state::AppState,
    visual::VisualSurface,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Примитивный CLI парсинг — без clap, чтобы не тянуть лишнего.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = parse_args(&args)?;

    let mut terminal = enter_terminal()?;
    let result = run_app(&mut terminal, cfg).await;
    leave_terminal(&mut terminal)?;
    result
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
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

fn leave_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cli: Cli,
) -> Result<()> {
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

    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(100);
    let mut dirty = true;

    loop {
        if dirty {
            terminal.draw(|frame| surface.render(frame, &state.visual_state()))?;
            dirty = false;
        }

        if state.should_quit {
            break;
        }

        // Select между: keyboard event (polling), event от driver, tick.
        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
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
                    Some(_) => {
                        // Неизвестный StdioOutput variant — future-proof.
                    }
                    None => {
                        state.push_error("agent process exited unexpectedly".into());
                        dirty = true;
                        state.should_quit = true;
                    }
                }
            }

            _ = tokio::time::sleep(timeout) => {
                if state.advance_spinner() {
                    dirty = true;
                }
                last_tick = Instant::now();

                // Параллельно poll'им клавиатуру — неблокирующий event::poll
                // не работает напрямую с async, поэтому опрашиваем в tick.
                while event::poll(Duration::from_millis(0))? {
                    let term_event = event::read()?;
                    if handle_term_event(&mut state, &mut driver, term_event).await? {
                        dirty = true;
                    }
                }
            }
        }
    }

    // Graceful shutdown ядра.
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
                    KeyCode::Char('n')
                    | KeyCode::Char('N')
                    | KeyCode::Esc => {
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
        CTerm::Mouse(me) => {
            use crossterm::event::MouseEventKind as K;
            match me.kind {
                K::ScrollUp => {
                    state.scroll_up(3);
                    return Ok(true);
                }
                K::ScrollDown => {
                    state.scroll_down(3);
                    return Ok(true);
                }
                _ => {}
            }
        }
        CTerm::Resize(_, _) => return Ok(true),
        _ => {}
    }
    Ok(false)
}
