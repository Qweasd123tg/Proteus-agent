//! Terminal UI клиент для modular-agent.
//!
//! Запускает ядро как subprocess через `agent server stdio`, читает поток
//! `AppServerEvent`, шлёт user input как `StdioRequest::Send`. Визуал на
//! ratatui/crossterm. Клиент depend только на `agent-contracts`, не на
//! самом ядре — границa client/core проведена через wire protocol.

mod bottom_pane;
mod cards;
mod driver;
mod history_insert;
mod inline_terminal;
mod live_preview;
mod markdown;
mod session_picker;
mod slash_commands;
mod state;
mod terminal_surface;
mod transcript;
mod visual;

use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

use agent_contracts::{
    app_protocol::{StdioOutput, StdioRequest},
    domain::{
        CallId, Event as DomainEvent, EventEnvelope, TokenUsageSnapshot, ToolCall, ToolResult,
        TurnId,
    },
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};
use anyhow::{Context, Result};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event as CTerm, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute, queue,
    terminal::{
        BeginSynchronizedUpdate, Clear as TerminalClear, ClearType, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};
use serde::Deserialize;

use crate::{
    driver::{AgentDriver, DriverConfig},
    inline_terminal::InlineTerminalState,
    session_picker::ResumePickerItem,
    state::AppState,
    terminal_surface::TerminalSurface,
    visual::{
        ReasoningDisplayMode, ToolCard, ToolStatus, VisualMessage, VisualSurface, compact_value,
    },
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

fn install_panic_hook() {
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableBracketedPaste,
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
    profile: Option<String>,
}

fn parse_args(args: &[String]) -> Result<Cli> {
    let mut agent_bin = None;
    let mut config_path = None;
    let mut cwd = None;
    let mut profile = None;
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--profile" | "-p" => {
                profile = iter
                    .next()
                    .map(ToOwned::to_owned)
                    .context("--profile requires value")
                    .ok();
            }
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
        profile,
    })
}

fn print_help() {
    eprintln!(
        "agent-tui — terminal UI for modular-agent\n\
         \n\
         usage:\n\
           agent-tui [--profile NAME] [--agent-bin PATH] [--config PATH] [--cwd PATH]\n\
         \n\
         options:\n\
           --profile, -p NAME  load ~/.config/agent-qweasd123tg/profiles/NAME.toml\n\
           --agent-bin PATH    path to the modular-agent binary (default: in $PATH)\n\
           --config PATH       path to agent config (toml or json)\n\
           --cwd PATH          workspace directory for the agent (default: current dir)\n\
           --help, -h          show this help"
    );
}

#[derive(Debug, Default, Deserialize)]
struct TuiProfileConfig {
    agent_bin: Option<PathBuf>,
    config: Option<PathBuf>,
    cwd: Option<PathBuf>,
}

fn apply_profile(cli: Cli) -> Result<Cli> {
    let Some(profile) = cli.profile.as_deref() else {
        return Ok(cli);
    };
    let profile_path = profile_path(profile)?;
    apply_profile_file(cli, &profile_path)
}

fn apply_profile_file(cli: Cli, profile_path: &Path) -> Result<Cli> {
    let content = std::fs::read_to_string(&profile_path)
        .with_context(|| format!("failed to read TUI profile {}", profile_path.display()))?;
    let profile_config: TuiProfileConfig = toml::from_str(&content)
        .with_context(|| format!("failed to parse TUI profile {}", profile_path.display()))?;
    let profile_dir = profile_path.parent().unwrap_or_else(|| Path::new("."));

    Ok(Cli {
        agent_bin: cli.agent_bin.or_else(|| {
            profile_config
                .agent_bin
                .map(|path| resolve_profile_path(profile_dir, path))
        }),
        config_path: cli.config_path.or_else(|| {
            profile_config
                .config
                .map(|path| resolve_profile_path(profile_dir, path))
        }),
        cwd: cli.cwd.or_else(|| {
            profile_config
                .cwd
                .map(|path| resolve_profile_path(profile_dir, path))
        }),
        profile: cli.profile,
    })
}

fn profile_path(profile: &str) -> Result<PathBuf> {
    if profile.trim().is_empty() {
        anyhow::bail!("profile name must not be empty");
    }
    let path = PathBuf::from(profile);
    if path.components().count() != 1 || path.is_absolute() {
        anyhow::bail!("profile name must be a simple file stem, got '{profile}'");
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join(".config/agent-qweasd123tg/profiles")
        .join(format!("{profile}.toml")))
}

fn resolve_profile_path(profile_dir: &Path, path: PathBuf) -> PathBuf {
    let path = expand_home_path(&path);
    if path.is_absolute() {
        path
    } else {
        profile_dir.join(path)
    }
}

fn enter_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    // Основной чат живёт в normal screen: завершённые сообщения пишутся в
    // настоящий terminal scrollback, поэтому выделение мышью и wheel работают
    // так же, как в shell. Mouse capture и alternate scroll здесь не включаем.
    execute!(
        out,
        EnableBracketedPaste,
        MoveTo(0, 0),
        TerminalClear(ClearType::All),
        TerminalClear(ClearType::Purge)
    )?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

fn leave_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        TerminalClear(ClearType::FromCursorDown)
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, cli: Cli) -> Result<()> {
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

async fn handle_term_event(
    state: &mut AppState,
    driver: &mut AgentDriver,
    driver_config: &DriverConfig,
    canceled_turn_responses: &mut HashSet<String>,
    cancel_request_responses: &mut HashSet<String>,
    ev: CTerm,
) -> Result<bool> {
    match ev {
        CTerm::Key(key) if is_handled_key_event(key.kind) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('c') => {
                        if state.has_context_report() {
                            state.close_context_report();
                        } else if state.input_is_empty() {
                            state.arm_or_confirm_quit();
                        } else {
                            state.clear_input();
                        }
                        return Ok(true);
                    }
                    _ => {}
                }
            }

            if state.has_context_report() {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                        state.close_context_report();
                        return Ok(true);
                    }
                    KeyCode::Up => {
                        state.scroll_context_report_up(1);
                        return Ok(true);
                    }
                    KeyCode::Down => {
                        state.scroll_context_report_down(1);
                        return Ok(true);
                    }
                    KeyCode::PageUp => {
                        state.scroll_context_report_up(8);
                        return Ok(true);
                    }
                    KeyCode::PageDown => {
                        state.scroll_context_report_down(8);
                        return Ok(true);
                    }
                    _ => return Ok(false),
                }
            }

            // Если открыт picker — клавиши управляют выбором session.
            if state.has_resume_picker() {
                match key.code {
                    KeyCode::Enter => {
                        if let Some(session_dir) = state.selected_resume_session() {
                            resume_session_dir(state, driver, driver_config, session_dir).await?;
                            canceled_turn_responses.clear();
                            cancel_request_responses.clear();
                        }
                        return Ok(true);
                    }
                    KeyCode::Esc => {
                        state.close_resume_picker();
                        return Ok(true);
                    }
                    KeyCode::Backspace => {
                        state.backspace_resume_query();
                        return Ok(true);
                    }
                    KeyCode::Char(ch) => {
                        state.type_resume_query_char(ch);
                        return Ok(true);
                    }
                    KeyCode::Tab => {
                        state.move_resume_selection_down(1);
                        return Ok(true);
                    }
                    KeyCode::BackTab => {
                        state.move_resume_selection_up(1);
                        return Ok(true);
                    }
                    KeyCode::Up => {
                        state.move_resume_selection_up(1);
                        return Ok(true);
                    }
                    KeyCode::Down => {
                        state.move_resume_selection_down(1);
                        return Ok(true);
                    }
                    KeyCode::PageUp => {
                        state.move_resume_selection_up(5);
                        return Ok(true);
                    }
                    KeyCode::PageDown => {
                        state.move_resume_selection_down(5);
                        return Ok(true);
                    }
                    _ => return Ok(false),
                }
            }

            // Если показан approval — обрабатываем y/n/p и те же клавиши в RU раскладке.
            if state.has_pending_approval() {
                match key.code {
                    KeyCode::Char('y')
                    | KeyCode::Char('Y')
                    | KeyCode::Char('1')
                    | KeyCode::Char('н')
                    | KeyCode::Char('Н') => {
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
                    KeyCode::Char('p')
                    | KeyCode::Char('P')
                    | KeyCode::Char('2')
                    | KeyCode::Char('з')
                    | KeyCode::Char('З') => {
                        if let Some(id) = state.take_pending_approval_id() {
                            driver
                                .send(&StdioRequest::Approval {
                                    id: None,
                                    approval_id: id,
                                    approved: true,
                                    note: None,
                                    cache:
                                        agent_contracts::contracts::ApprovalCacheScope::ExactCall,
                                })
                                .await?;
                            return Ok(true);
                        }
                    }
                    KeyCode::Char('n')
                    | KeyCode::Char('N')
                    | KeyCode::Char('3')
                    | KeyCode::Char('т')
                    | KeyCode::Char('Т')
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
                    if state.complete_partial_slash_suggestion() {
                        return Ok(true);
                    } else if state.pending_model {
                        if let Some(submission) = state.take_input_for_slash_command() {
                            handle_slash_command(
                                state,
                                driver,
                                driver_config,
                                canceled_turn_responses,
                                cancel_request_responses,
                                &submission.text,
                            )
                            .await?;
                        } else if !state.input_is_empty() {
                            state.reject_send_while_busy();
                        }
                        return Ok(true);
                    } else if let Some(submission) = state.take_input_for_send() {
                        if submission.text.starts_with('/') {
                            handle_slash_command(
                                state,
                                driver,
                                driver_config,
                                canceled_turn_responses,
                                cancel_request_responses,
                                &submission.text,
                            )
                            .await?;
                        } else {
                            let turn_id = state.next_turn_id();
                            driver
                                .send(&StdioRequest::Send {
                                    id: Some(turn_id.clone()),
                                    text: submission.text.clone(),
                                })
                                .await?;
                            state.mark_user_sent(submission.text, submission.paste_ranges, turn_id);
                        }
                        return Ok(true);
                    }
                }
                KeyCode::Tab => {
                    if state.has_slash_suggestions() {
                        state.complete_slash_suggestion();
                        return Ok(true);
                    }
                }
                KeyCode::BackTab => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_prev();
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
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_prev();
                        return Ok(true);
                    }
                }
                KeyCode::PageDown => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_next();
                        return Ok(true);
                    }
                }
                KeyCode::Up => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_prev();
                        return Ok(true);
                    }
                }
                KeyCode::Down => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_next();
                        return Ok(true);
                    }
                }
                KeyCode::Right => {
                    if state.complete_slash_suggestion() {
                        return Ok(true);
                    }
                }
                KeyCode::Esc => {
                    if state.pending_model
                        && request_cancel(
                            state,
                            driver,
                            canceled_turn_responses,
                            cancel_request_responses,
                        )
                        .await?
                    {
                        return Ok(true);
                    }
                }
                _ => {}
            }
        }
        CTerm::Paste(text) => {
            state.paste_text(&text);
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

fn reset_normal_screen(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    header_printed: &mut bool,
    inline_terminal: &mut InlineTerminalState,
) -> Result<()> {
    TerminalSurface::new(terminal).clear_normal_screen(true)?;
    state.rewind_scrollback();
    *header_printed = false;
    inline_terminal.reset();
    Ok(())
}

fn redraw(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    surface: &VisualSurface,
    state: &mut AppState,
    scrollback_header_printed: &mut bool,
    inline_terminal: &mut InlineTerminalState,
    picker_alt_screen: &mut bool,
) -> Result<()> {
    queue!(terminal.backend_mut(), Hide, BeginSynchronizedUpdate)?;
    let result = (|| -> Result<()> {
        if state.has_fullscreen_overlay() {
            if !*picker_alt_screen {
                inline_terminal.enter_overlay(terminal)?;
                execute!(terminal.backend_mut(), EnterAlternateScreen)?;
                terminal.clear()?;
                *picker_alt_screen = true;
            }
            terminal.draw(|frame| surface.render_inline(frame, &state.visual_state()))?;
        } else {
            if *picker_alt_screen {
                execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                *picker_alt_screen = false;
                inline_terminal.leave_overlay();
            }
            inline_terminal.draw_normal(terminal, state, scrollback_header_printed)?;
        }
        Ok(())
    })();
    let finish_result = queue!(terminal.backend_mut(), EndSynchronizedUpdate, Show)
        .and_then(|_| std::io::Write::flush(terminal.backend_mut()));
    result?;
    finish_result?;
    Ok(())
}

fn is_handled_key_event(kind: KeyEventKind) -> bool {
    matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

async fn handle_slash_command(
    state: &mut AppState,
    driver: &mut AgentDriver,
    driver_config: &DriverConfig,
    canceled_turn_responses: &mut HashSet<String>,
    cancel_request_responses: &mut HashSet<String>,
    text: &str,
) -> Result<()> {
    let command = text.trim();
    let (name, rest) = command
        .split_once(char::is_whitespace)
        .map(|(name, rest)| (name, rest.trim()))
        .unwrap_or((command, ""));
    match name {
        "/help" => {
            state.push_system(
                "/help commands: /clear, /cancel, /resume [session-dir], /session, /context, /reasoning [hidden|summary|expanded], /quit",
            );
        }
        "/clear" => {
            driver
                .send(&StdioRequest::ClearHistory { id: None })
                .await?;
            state.clear_transcript();
        }
        "/cancel" => {
            if !request_cancel(
                state,
                driver,
                canceled_turn_responses,
                cancel_request_responses,
            )
            .await?
            {
                state.push_system("No active turn to cancel.");
            }
        }
        "/quit" | "/exit" => {
            state.should_quit = true;
        }
        "/session" => {
            let message = state
                .session_dir()
                .map(|path| format!("session: {}", path.display()))
                .unwrap_or_else(|| "session: not persisted".to_owned());
            state.push_system(message);
        }
        "/context" => state.open_context_report(),
        "/reasoning" => handle_reasoning_command(state, rest),
        "/resume" => {
            if state.pending_model {
                state.push_error("cancel active turn before resume".to_owned());
            } else if rest.is_empty() {
                let sessions =
                    list_resume_sessions(driver_config.config_path.as_deref(), state.cwd())?;
                if sessions.is_empty() {
                    state.push_system("No sessions found for this workspace.");
                } else {
                    state.open_resume_picker(sessions);
                }
            } else {
                resume_session(state, driver, driver_config, rest).await?;
                canceled_turn_responses.clear();
                cancel_request_responses.clear();
            }
        }
        _ => {
            state.push_error(format!("unknown command: {name}. Try /help"));
        }
    }
    Ok(())
}

fn handle_reasoning_command(state: &mut AppState, rest: &str) {
    match rest {
        "" => state.open_reasoning_report(),
        "hidden" => state.set_reasoning_mode(ReasoningDisplayMode::Hidden),
        "summary" => state.set_reasoning_mode(ReasoningDisplayMode::Summary),
        "expanded" => state.set_reasoning_mode(ReasoningDisplayMode::Expanded),
        _ => state.push_error("usage: /reasoning [hidden|summary|expanded]".to_owned()),
    }
}

async fn resume_session(
    state: &mut AppState,
    driver: &mut AgentDriver,
    driver_config: &DriverConfig,
    raw_path: &str,
) -> Result<()> {
    let session_dir = resolve_session_dir(raw_path)?;
    resume_session_dir(state, driver, driver_config, session_dir).await
}

async fn resume_session_dir(
    state: &mut AppState,
    driver: &mut AgentDriver,
    driver_config: &DriverConfig,
    session_dir: PathBuf,
) -> Result<()> {
    let history = load_session_history(&session_dir)?;
    let context_usage = load_session_context_usage(
        &session_dir,
        driver_config.config_path.as_deref(),
        state.cwd(),
    )?;
    let mut resumed_config = driver_config.clone();
    resumed_config.resume_session = Some(session_dir.clone());

    driver.shutdown().await?;
    *driver = AgentDriver::spawn(resumed_config).await?;
    state.reset_after_resume_with_history(session_dir, history);
    state.restore_context_usage(context_usage);
    Ok(())
}

fn load_session_context_usage(
    session_dir: &Path,
    config_path: Option<&Path>,
    cwd: &Path,
) -> Result<Vec<(TokenUsageSnapshot, Option<TurnId>)>> {
    let Some(session_id) = read_session_id_string(session_dir)? else {
        return Ok(Vec::new());
    };
    let mut snapshots = Vec::new();
    for event_log_path in candidate_event_log_paths(config_path, cwd) {
        let content = match std::fs::read_to_string(&event_log_path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", event_log_path.display()));
            }
        };
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(envelope) = serde_json::from_str::<EventEnvelope>(line) else {
                continue;
            };
            if envelope.session_id.to_string() != session_id {
                continue;
            }
            if let DomainEvent::TokenUsageUpdated { usage } = envelope.event {
                snapshots.push((usage, envelope.turn_id));
            }
        }
    }
    Ok(snapshots)
}

fn read_session_id_string(session_dir: &Path) -> Result<Option<String>> {
    let metadata_path = session_dir.join("session.json");
    let content = match std::fs::read_to_string(&metadata_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", metadata_path.display()));
        }
    };
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", metadata_path.display()))?;
    Ok(value
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned))
}

fn candidate_event_log_paths(config_path: Option<&Path>, cwd: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in candidate_config_roots(config_path) {
        paths.push(root.join(".agent-claude-pack/events.jsonl"));
        paths.push(root.join(".agent/events.jsonl"));
    }
    paths.push(cwd.join(".agent-claude-pack/events.jsonl"));
    paths.push(cwd.join(".agent/events.jsonl"));
    dedup_paths(paths)
}

fn load_session_history(session_dir: &Path) -> Result<Vec<VisualMessage>> {
    let messages_path = session_dir.join("messages.jsonl");
    let content = match std::fs::read_to_string(&messages_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", messages_path.display()));
        }
    };

    let mut output = Vec::new();
    let mut tool_calls = HashMap::<CallId, ToolCall>::new();
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let message: CanonicalMessage = serde_json::from_str(line).with_context(|| {
            format!(
                "failed to parse {} line {}",
                messages_path.display(),
                index + 1
            )
        })?;
        append_history_message(&mut output, &mut tool_calls, message);
    }
    Ok(output)
}

fn append_history_message(
    output: &mut Vec<VisualMessage>,
    tool_calls: &mut HashMap<CallId, ToolCall>,
    message: CanonicalMessage,
) {
    let mut text_parts = Vec::new();
    for part in message.parts {
        match part {
            ContentPart::Text { text } | ContentPart::ReasoningSummary { text } => {
                if !text.trim().is_empty() {
                    text_parts.push(text);
                }
            }
            ContentPart::ToolCall { call } => {
                tool_calls.insert(call.id.clone(), call);
            }
            ContentPart::ToolResult { result } => {
                output.push(history_tool_message(tool_calls, result));
            }
            ContentPart::FileRef { path, content } => {
                let text = match content {
                    Some(content) => format!("{}:\n{}", path.display(), content),
                    None => path.display().to_string(),
                };
                text_parts.push(text);
            }
            ContentPart::Patch { patch } => {
                text_parts.push(patch.content);
            }
            ContentPart::Context { .. } => {}
            _ => {}
        }
    }

    if text_parts.is_empty() {
        return;
    }
    let text = text_parts.join("\n\n");
    let visual = match message.role {
        MessageRole::User => VisualMessage::user(text),
        MessageRole::Assistant => VisualMessage::assistant(text),
        MessageRole::System | MessageRole::Developer => VisualMessage::system(text),
        MessageRole::Tool => return,
        _ => return,
    };
    output.push(visual);
}

fn message_text(message: &CanonicalMessage) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } | ContentPart::ReasoningSummary { text } => {
                Some(text.as_str())
            }
            ContentPart::FileRef { path, .. } => path.to_str(),
            ContentPart::Patch { patch } => Some(patch.content.as_str()),
            _ => None,
        })
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn history_tool_message(
    tool_calls: &HashMap<CallId, ToolCall>,
    result: ToolResult,
) -> VisualMessage {
    let call = tool_calls.get(&result.call_id);
    VisualMessage::tool(ToolCard {
        call_id: result.call_id.clone(),
        name: call
            .map(|call| call.name.clone())
            .unwrap_or_else(|| "tool".to_owned()),
        args_summary: call
            .map(|call| compact_value(&call.args))
            .unwrap_or_default(),
        status: if result.ok {
            ToolStatus::Ok
        } else {
            ToolStatus::Err
        },
        output_preview: history_tool_preview(&result),
    })
}

fn history_tool_preview(result: &ToolResult) -> String {
    if let Some(error) = &result.error {
        return error.clone();
    }
    let mut out = String::new();
    for ch in result.output.chars() {
        match ch {
            '\t' => out.push_str("  "),
            '\r' => {}
            other => out.push(other),
        }
        if out.chars().count() >= 160 {
            break;
        }
    }
    out
}

fn list_resume_sessions(config_path: Option<&Path>, cwd: &Path) -> Result<Vec<ResumePickerItem>> {
    let mut sessions = Vec::new();
    for root in candidate_config_roots(config_path) {
        let sessions_dir = root.join("sessions").join(encode_workspace_path(cwd));
        append_resume_sessions_from_dir(&sessions_dir, &mut sessions)?;
    }

    sessions.sort_by(|left, right| right.updated.cmp(&left.updated));
    Ok(sessions)
}

fn candidate_config_roots(config_path: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(config_path) = config_path {
        push_config_root_candidates(&mut roots, expand_home_path(config_path));
    } else if let Some(default_path) = default_config_path() {
        push_config_root_candidates(&mut roots, expand_home_path(&default_path));
    }
    dedup_paths(roots)
}

fn push_config_root_candidates(roots: &mut Vec<PathBuf>, path: PathBuf) {
    roots.push(config_store_root(&path));

    if path.file_name().and_then(|name| name.to_str()) == Some("configs")
        && let Some(parent) = path.parent()
    {
        roots.push(parent.to_path_buf());
    }

    if let Some(parent) = path.parent() {
        roots.push(parent.to_path_buf());
        if parent.file_name().and_then(|name| name.to_str()) == Some("configs")
            && let Some(root) = parent.parent()
        {
            roots.push(root.to_path_buf());
        }
    }
}

fn dedup_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if !out.iter().any(|existing| existing == &path) {
            out.push(path);
        }
    }
    out
}

fn append_resume_sessions_from_dir(
    sessions_dir: &Path,
    sessions: &mut Vec<ResumePickerItem>,
) -> Result<()> {
    let entries = match std::fs::read_dir(sessions_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", sessions_dir.display()));
        }
    };

    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read {}", sessions_dir.display()))?;
        let session_dir = entry.path();
        if !entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", session_dir.display()))?
            .is_dir()
        {
            continue;
        }
        let metadata_path = session_dir.join("session.json");
        let messages_path = session_dir.join("messages.jsonl");
        if !metadata_path.is_file() {
            continue;
        }
        if sessions
            .iter()
            .any(|item| item.session_dir.as_path() == session_dir.as_path())
        {
            continue;
        }

        let title = session_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("session")
            .to_owned();
        let created = file_created_time(&metadata_path)
            .or_else(|| file_created_time(&session_dir))
            .or_else(|| file_modified_time(&metadata_path))
            .or_else(|| file_modified_time(&session_dir));
        let modified = file_modified_time(&messages_path)
            .or_else(|| file_modified_time(&metadata_path))
            .or_else(|| file_modified_time(&session_dir));
        let conversation = session_conversation_title(&messages_path)
            .unwrap_or_else(|| "empty session".to_owned());
        sessions.push(ResumePickerItem {
            session_dir,
            id: title,
            created: format_time_ago(created),
            updated_label: format_time_ago(modified),
            branch: "-".to_owned(),
            conversation,
            updated: modified,
        });
    }
    Ok(())
}

fn session_conversation_title(messages_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(messages_path).ok()?;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let message = serde_json::from_str::<CanonicalMessage>(line).ok()?;
        if matches!(message.role, MessageRole::User) {
            let text = message_text(&message);
            if !text.trim().is_empty() {
                return Some(text.replace('\n', " "));
            }
        }
    }
    None
}

fn file_created_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.created())
        .ok()
}

fn file_modified_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn format_time_ago(time: Option<SystemTime>) -> String {
    let Some(time) = time else {
        return "unknown".to_owned();
    };
    let Ok(elapsed) = SystemTime::now().duration_since(time) else {
        return "just now".to_owned();
    };
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        "just now".to_owned()
    } else if seconds < 3_600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3_600)
    } else {
        format!("{}d ago", seconds / 86_400)
    }
}

fn default_config_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AGENT_CONFIG_PATH") {
        return Some(PathBuf::from(path));
    }
    if let Some(config_home) = std::env::var_os("AGENT_CONFIG_HOME") {
        return Some(PathBuf::from(config_home).join("configs"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        return Some(PathBuf::from(home).join(".config/agent-qweasd123tg/configs"));
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(|xdg_config_home| PathBuf::from(xdg_config_home).join("agent-qweasd123tg/configs"))
}

fn config_store_root(path: &Path) -> PathBuf {
    if path.is_dir() {
        return path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf());
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if parent.file_name().and_then(|name| name.to_str()) == Some("configs")
        && let Some(root) = parent.parent()
    {
        return root.to_path_buf();
    }
    parent.to_path_buf()
}

fn encode_workspace_path(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let parts = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(sanitize_path_part(&part.to_string_lossy())),
            _ => None,
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    if parts.is_empty() {
        "root".to_owned()
    } else {
        parts.join("|")
    }
}

fn sanitize_path_part(input: &str) -> String {
    let mut out = String::new();
    for ch in input.trim().chars() {
        if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_owned()
}

fn resolve_session_dir(raw_path: &str) -> Result<PathBuf> {
    let path = expand_home(raw_path.trim());
    let metadata = std::fs::metadata(&path)
        .with_context(|| format!("failed to inspect session path {}", path.display()))?;
    if metadata.is_dir() {
        return Ok(path);
    }
    if path.file_name().and_then(|name| name.to_str()) == Some("messages.jsonl") {
        return path
            .parent()
            .map(PathBuf::from)
            .context("messages.jsonl path has no parent session dir");
    }
    anyhow::bail!("resume path is not a session directory: {}", path.display())
}

fn expand_home(path: &str) -> PathBuf {
    expand_home_path(Path::new(path))
}

fn expand_home_path(path: &Path) -> PathBuf {
    let Some(path_str) = path.to_str() else {
        return path.to_path_buf();
    };
    if path_str == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home);
    }
    if let Some(rest) = path_str.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

async fn request_cancel(
    state: &mut AppState,
    driver: &mut AgentDriver,
    canceled_turn_responses: &mut HashSet<String>,
    cancel_request_responses: &mut HashSet<String>,
) -> Result<bool> {
    let Some(turn_id) = state.active_turn_id().map(str::to_owned) else {
        return Ok(false);
    };
    let request_id = format!("cancel-{turn_id}");
    driver
        .send(&StdioRequest::Cancel {
            id: Some(request_id.clone()),
            target_id: turn_id.clone(),
        })
        .await?;
    canceled_turn_responses.insert(turn_id);
    cancel_request_responses.insert(request_id);
    state.mark_cancel_requested();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual::VisualRole;

    #[test]
    fn resolve_session_dir_accepts_directory() {
        let dir = tempfile::tempdir().expect("session dir");

        let resolved = resolve_session_dir(&dir.path().display().to_string()).expect("resolved");

        assert_eq!(resolved, dir.path());
    }

    #[test]
    fn resolve_session_dir_accepts_messages_jsonl_file() {
        let dir = tempfile::tempdir().expect("session dir");
        let messages = dir.path().join("messages.jsonl");
        std::fs::write(&messages, "").expect("messages file");

        let resolved = resolve_session_dir(&messages.display().to_string()).expect("resolved");

        assert_eq!(resolved, dir.path());
    }

    #[test]
    fn list_resume_sessions_reads_current_workspace_sessions() {
        let config_root = tempfile::tempdir().expect("config root");
        let config_dir = config_root.path().join("configs");
        std::fs::create_dir(&config_dir).expect("config dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let sessions_dir = config_root
            .path()
            .join("sessions")
            .join(encode_workspace_path(cwd.path()));
        let valid_session = sessions_dir.join("1234567890");
        let invalid_session = sessions_dir.join("9999999999");
        std::fs::create_dir_all(&valid_session).expect("valid session dir");
        std::fs::create_dir_all(&invalid_session).expect("invalid session dir");
        std::fs::write(valid_session.join("session.json"), "{}").expect("session metadata");
        std::fs::write(valid_session.join("messages.jsonl"), "").expect("messages");

        let sessions = list_resume_sessions(Some(&config_dir), cwd.path()).expect("sessions");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "1234567890");
        assert_eq!(sessions[0].session_dir, valid_session);
    }

    #[test]
    fn load_session_history_renders_text_and_tool_messages() {
        let dir = tempfile::tempdir().expect("session dir");
        let call = ToolCall::new("call-1", "list_dir", serde_json::json!({"path":"."}));
        let messages = vec![
            CanonicalMessage::text(MessageRole::User, "hello"),
            CanonicalMessage::new(
                MessageRole::Assistant,
                vec![ContentPart::ToolCall { call: call.clone() }],
            ),
            CanonicalMessage::new(
                MessageRole::Tool,
                vec![ContentPart::ToolResult {
                    result: ToolResult::ok(call.id.clone(), "file  a.md"),
                }],
            )
            .with_tool_call_id(call.id.clone()),
            CanonicalMessage::text(MessageRole::Assistant, "done"),
        ];
        let content = messages
            .into_iter()
            .map(|message| serde_json::to_string(&message).expect("message json"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.path().join("messages.jsonl"), content).expect("messages");

        let history = load_session_history(dir.path()).expect("history");

        assert_eq!(history.len(), 3);
        assert!(matches!(history[0].role, VisualRole::User));
        assert_eq!(history[0].text, "hello");
        assert!(matches!(history[1].role, VisualRole::Tool));
        let tool = history[1].tool.as_ref().expect("tool card");
        assert_eq!(tool.name, "list_dir");
        assert_eq!(tool.output_preview, "file  a.md");
        assert!(matches!(history[2].role, VisualRole::Assistant));
        assert_eq!(history[2].text, "done");
    }

    #[test]
    fn load_session_context_usage_reads_token_events_for_session() {
        let session_dir = tempfile::tempdir().expect("session dir");
        let cwd = tempfile::tempdir().expect("cwd");
        let session_id = agent_contracts::domain::new_session_id();
        let other_session_id = agent_contracts::domain::new_session_id();
        let thread_id = agent_contracts::domain::new_thread_id();
        let turn_id = agent_contracts::domain::new_turn_id();
        std::fs::write(
            session_dir.path().join("session.json"),
            serde_json::json!({
                "schema_version": 1,
                "session_id": session_id,
            })
            .to_string(),
        )
        .expect("metadata");
        let event_dir = cwd.path().join(".agent");
        std::fs::create_dir(&event_dir).expect("event dir");
        let wanted = EventEnvelope::new(
            agent_contracts::domain::EventContext::new(session_id, thread_id, Some(turn_id)),
            1,
            DomainEvent::TokenUsageUpdated {
                usage: TokenUsageSnapshot::new(
                    agent_contracts::domain::ModelRef::new("test", "model"),
                    123,
                    Vec::new(),
                ),
            },
        );
        let ignored = EventEnvelope::new(
            agent_contracts::domain::EventContext::new(other_session_id, thread_id, Some(turn_id)),
            2,
            DomainEvent::TokenUsageUpdated {
                usage: TokenUsageSnapshot::new(
                    agent_contracts::domain::ModelRef::new("test", "other"),
                    999,
                    Vec::new(),
                ),
            },
        );
        let lines = [ignored, wanted]
            .into_iter()
            .map(|event| serde_json::to_string(&event).expect("event json"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(event_dir.join("events.jsonl"), lines).expect("event log");

        let usage =
            load_session_context_usage(session_dir.path(), None, cwd.path()).expect("usage");

        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].0.estimated_input_tokens, 123);
        assert_eq!(usage[0].1, Some(turn_id));
    }

    #[test]
    fn profile_file_fills_missing_launcher_fields() {
        let dir = tempfile::tempdir().expect("profile dir");
        let profile_path = dir.path().join("claude.toml");
        std::fs::write(
            &profile_path,
            r#"
agent_bin = "bin/agent"
config = "~/agent-config/configs"
cwd = "workspace"
"#,
        )
        .expect("profile");
        let cli = Cli {
            agent_bin: None,
            config_path: Some(PathBuf::from("/explicit/config")),
            cwd: None,
            profile: Some("claude".to_owned()),
        };

        let cli = apply_profile_file(cli, &profile_path).expect("applied profile");

        assert_eq!(cli.agent_bin, Some(dir.path().join("bin/agent")));
        assert_eq!(cli.config_path, Some(PathBuf::from("/explicit/config")));
        assert_eq!(cli.cwd, Some(dir.path().join("workspace")));
    }

    #[test]
    fn handled_key_events_include_repeat_events() {
        assert!(is_handled_key_event(KeyEventKind::Press));
        assert!(is_handled_key_event(KeyEventKind::Repeat));
        assert!(!is_handled_key_event(KeyEventKind::Release));
    }
}
