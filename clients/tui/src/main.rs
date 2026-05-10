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
mod markdown;
mod session_picker;
mod slash_commands;
mod state;
mod terminal_surface;
mod transcript;
mod visual;

use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use agent_contracts::app_protocol::{StdioOutput, StdioRequest};
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
    commands::{handle_slash_command, request_cancel, resume_session_dir},
    driver::{AgentDriver, DriverConfig},
    inline_terminal::InlineTerminalState,
    state::AppState,
    terminal_surface::TerminalSurface,
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
            terminal.draw(|frame| surface.render_overlay(frame, &state.visual_state()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
