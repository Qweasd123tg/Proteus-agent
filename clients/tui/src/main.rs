//! Terminal UI клиент для modular-agent.
//!
//! Запускает ядро как subprocess через `agent server stdio`, читает поток
//! `AppServerEvent`, шлёт user input как `StdioRequest::Send`. Визуал на
//! ratatui/crossterm. Клиент depend только на `agent-contracts`, не на
//! самом ядре — границa client/core проведена через wire protocol.

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

use std::{collections::HashSet, path::PathBuf, time::Duration};

use agent_contracts::app_protocol::StdioOutput;
use anyhow::Result;
use crossterm::{
    event::{self, Event as CTerm},
    execute,
    terminal::LeaveAlternateScreen,
};

use crate::{
    driver::{AgentDriver, DriverConfig},
    inline_terminal::InlineTerminalState,
    input::handle_term_event,
    profiles::{Cli, apply_profile, parse_args},
    state::AppState,
    terminal_host::{
        TuiTerminal, enter_terminal, install_panic_hook, leave_terminal, redraw,
        reset_normal_screen,
    },
    visual::VisualSurface,
};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);
const ACTIVITY_STATUS_INTERVAL: Duration = Duration::from_millis(200);

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

async fn run_app(terminal: &mut TuiTerminal, cli: Cli) -> Result<()> {
    let cwd = cli
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let driver_config = DriverConfig {
        agent_bin: cli.agent_bin.clone(),
        config_path: cli.config_path.clone(),
        cwd: Some(cwd.clone()),
        resume_session: None,
    };
    let mut driver = AgentDriver::spawn(driver_config.clone()).await?;

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

    let mut frame_tick = tokio::time::interval(FRAME_INTERVAL);
    frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut activity_status_tick = tokio::time::interval(ACTIVITY_STATUS_INTERVAL);
    activity_status_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut canceled_turn_responses = HashSet::<String>::new();
    let mut cancel_request_responses = HashSet::<String>::new();
    let mut scrollback_header_printed = false;
    let mut inline_terminal = InlineTerminalState::default();
    let mut picker_alt_screen = false;
    let mut startup_ready = false;
    let mut dirty = false;

    loop {
        if state.should_quit && !dirty {
            break;
        }

        tokio::select! {
            biased;

            output = driver.events.recv() => {
                match output {
                    Some(StdioOutput::Event { event }) => {
                        startup_ready = true;
                        let header_before = state.header_identity();
                        state.ingest(*event);
                        if scrollback_header_printed && state.header_identity() != header_before {
                            if picker_alt_screen {
                                terminal.clear()?;
                            } else {
                                reset_normal_screen(
                                    terminal,
                                    &mut state,
                                    &mut scrollback_header_printed,
                                    &mut inline_terminal,
                                )?;
                            }
                        }
                        dirty = true;
                    }
                    Some(StdioOutput::Response { id, ok, error, .. }) => {
                        startup_ready = true;
                        if id
                            .as_ref()
                            .is_some_and(|id| cancel_request_responses.remove(id))
                        {
                            dirty = true;
                            continue;
                        }

                        if id
                            .as_ref()
                            .is_some_and(|id| canceled_turn_responses.remove(id))
                            && !ok
                            && error
                                .as_deref()
                                .is_none_or(|error| error.contains("turn canceled"))
                        {
                            dirty = true;
                            continue;
                        }

                        if !ok {
                            state.push_error(error.unwrap_or_else(|| "request failed".into()));
                            dirty = true;
                        }
                    }
                    Some(_) => {}
                    None => {
                        startup_ready = true;
                        state.push_error("agent process exited unexpectedly".into());
                        dirty = true;
                        state.should_quit = true;
                    }
                }
            }

            term_event = input_rx.recv() => {
                match term_event {
                    Some(ev) => {
                        if matches!(ev, CTerm::Resize(_, _)) {
                            inline_terminal.mark_resize_reflow_pending();
                            if picker_alt_screen {
                                terminal.clear()?;
                            }
                            dirty = true;
                            continue;
                        }
                        if handle_term_event(
                            &mut state,
                            &mut driver,
                            &driver_config,
                            &mut canceled_turn_responses,
                            &mut cancel_request_responses,
                            ev,
                        )
                        .await?
                        {
                            dirty = true;
                        }
                    }
                    None => {
                        // Input thread завершился — редкий случай, продолжаем.
                    }
                }
            }

            _ = frame_tick.tick() => {
                if dirty && startup_ready {
                    redraw(
                        terminal,
                        &surface,
                        &mut state,
                        &mut scrollback_header_printed,
                        &mut inline_terminal,
                        &mut picker_alt_screen,
                    )?;
                    dirty = false;
                }
            }

            _ = activity_status_tick.tick() => {
                if state.advance_activity_status() {
                    dirty = true;
                }
            }
        }
    }

    let _ = driver.shutdown().await;
    if picker_alt_screen {
        let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    }
    Ok(())
}
