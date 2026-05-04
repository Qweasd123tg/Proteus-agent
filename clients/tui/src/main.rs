//! Terminal UI клиент для modular-agent.
//!
//! Запускает ядро как subprocess через `agent server stdio`, читает поток
//! `AppServerEvent`, шлёт user input как `StdioRequest::Send`. Визуал на
//! ratatui/crossterm. Клиент depend только на `agent-contracts`, не на
//! самом ядре — границa client/core проведена через wire protocol.

mod driver;
mod markdown;
mod session_picker;
mod slash_commands;
mod state;
mod visual;

use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

use agent_contracts::{
    app_protocol::{StdioOutput, StdioRequest},
    domain::{CallId, ToolCall, ToolResult},
    model_standard::{CanonicalMessage, ContentPart, MessageRole},
};
use anyhow::{Context, Result};
use crossterm::{
    cursor::{MoveToColumn, MoveUp},
    event::{self, Event as CTerm, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::{Attribute, Color as CTermColor, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{
        Clear as TerminalClear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode,
    },
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    style::{Color as RColor, Modifier, Style},
    text::Line,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    driver::{AgentDriver, DriverConfig},
    session_picker::ResumePickerItem,
    state::AppState,
    visual::{
        ToolCard, ToolStatus, VisualMessage, VisualSurface, compact_value, inline_panel_lines,
        render_scrollback_header, render_scrollback_message,
    },
};

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
    // Основной чат живёт в normal screen: завершённые сообщения пишутся в
    // настоящий terminal scrollback, поэтому выделение мышью и wheel работают
    // так же, как в shell. Mouse capture и alternate scroll здесь не включаем.
    execute!(out, TerminalClear(ClearType::FromCursorDown))?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

fn leave_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
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

    let mut tick = tokio::time::interval(Duration::from_millis(120));
    let mut canceled_turn_responses = HashSet::<String>::new();
    let mut cancel_request_responses = HashSet::<String>::new();
    let mut scrollback_header_printed = false;
    let mut inline_panel = InlinePanelLayout::default();
    let mut picker_alt_screen = false;
    let mut dirty = true;

    loop {
        if dirty {
            if state.has_resume_picker() {
                if !picker_alt_screen {
                    if inline_panel.height > 0 {
                        clear_inline_panel(terminal, inline_panel)?;
                        inline_panel = InlinePanelLayout::default();
                    }
                    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
                    picker_alt_screen = true;
                }
                terminal.draw(|frame| surface.render_inline(frame, &state.visual_state()))?;
            } else {
                if picker_alt_screen {
                    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                    picker_alt_screen = false;
                    inline_panel = InlinePanelLayout::default();
                }
                if inline_panel.height > 0 {
                    clear_inline_panel(terminal, inline_panel)?;
                }
                flush_scrollback_messages(terminal, &mut state, &mut scrollback_header_printed)?;
                inline_panel = draw_inline_panel(terminal, &state)?;
            }
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
                    Some(StdioOutput::Response { id, ok, error, .. }) => {
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
                        state.push_error("agent process exited unexpectedly".into());
                        dirty = true;
                        state.should_quit = true;
                    }
                }
            }

            term_event = input_rx.recv() => {
                match term_event {
                    Some(ev) => {
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

            _ = tick.tick() => {
                if state.advance_spinner() {
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
                    } else if let Some(text) = state.take_input_for_send() {
                        if text.starts_with('/') {
                            handle_slash_command(
                                state,
                                driver,
                                driver_config,
                                canceled_turn_responses,
                                cancel_request_responses,
                                &text,
                            )
                            .await?;
                        } else {
                            let turn_id = state.next_turn_id();
                            driver
                                .send(&StdioRequest::Send {
                                    id: Some(turn_id.clone()),
                                    text: text.clone(),
                                })
                                .await?;
                            state.mark_user_sent(text, turn_id);
                        }
                        return Ok(true);
                    }
                }
                KeyCode::Tab => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_next();
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
                    state.scroll_up(5);
                    return Ok(true);
                }
                KeyCode::PageDown => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_next();
                        return Ok(true);
                    }
                    state.scroll_down(5);
                    return Ok(true);
                }
                // Wheel scroll через alternate-scroll mode приходит как
                // Up/Down arrows. Ловим и скроллим транскрипт на 1 строку.
                KeyCode::Up => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_prev();
                        return Ok(true);
                    }
                    state.scroll_up(1);
                    return Ok(true);
                }
                KeyCode::Down => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_next();
                        return Ok(true);
                    }
                    state.scroll_down(1);
                    return Ok(true);
                }
                KeyCode::Right => {
                    if state.complete_slash_suggestion() {
                        return Ok(true);
                    }
                }
                KeyCode::End => {
                    state.scroll_to_bottom();
                    return Ok(true);
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
        CTerm::Resize(_, _) => return Ok(true),
        _ => {}
    }
    Ok(false)
}

fn flush_scrollback_messages(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    header_printed: &mut bool,
) -> Result<()> {
    let messages = state.drain_scrollback_messages();
    if messages.is_empty() && *header_printed {
        return Ok(());
    }

    let size = terminal.size()?;
    let width = size.width.max(1) as usize;
    let render_width = width.saturating_sub(1).max(1);
    if !*header_printed {
        for line in render_scrollback_header(&state.visual_state(), render_width) {
            write_scrollback_line(terminal, &line, width)?;
        }
        *header_printed = true;
    }
    for message in messages {
        for line in render_scrollback_message(&message, render_width) {
            write_scrollback_line(terminal, &line, width)?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
struct InlinePanelLayout {
    height: u16,
    cursor_row: u16,
}

fn draw_inline_panel(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &AppState,
) -> Result<InlinePanelLayout> {
    let size = terminal.size()?;
    let width = size.width.max(1) as usize;
    let mut lines = inline_panel_lines(&state.visual_state(), width);
    let max_lines = size.height.saturating_sub(1) as usize;
    if lines.len() > max_lines {
        lines.drain(0..lines.len() - max_lines);
    }

    let panel_height = lines.len() as u16;
    for line in &lines {
        execute!(
            terminal.backend_mut(),
            TerminalClear(ClearType::CurrentLine),
            Print(truncate_terminal_line(line, width)),
            Print("\r\n")
        )?;
    }
    let cursor_row = lines.len().saturating_sub(3) as u16;
    let rows_from_after_panel = panel_height.saturating_sub(cursor_row);
    execute!(
        terminal.backend_mut(),
        MoveUp(rows_from_after_panel),
        MoveToColumn(
            (2 + state.visual_state().input.chars().count()).min(width.saturating_sub(1)) as u16
        )
    )?;
    Ok(InlinePanelLayout {
        height: panel_height,
        cursor_row,
    })
}

fn clear_inline_panel(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    layout: InlinePanelLayout,
) -> Result<()> {
    if layout.cursor_row > 0 {
        execute!(terminal.backend_mut(), MoveUp(layout.cursor_row))?;
    }
    execute!(
        terminal.backend_mut(),
        MoveToColumn(0),
        TerminalClear(ClearType::FromCursorDown)
    )?;
    Ok(())
}

fn write_scrollback_line(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    line: &Line<'_>,
    width: usize,
) -> Result<()> {
    execute!(
        terminal.backend_mut(),
        TerminalClear(ClearType::CurrentLine)
    )?;

    let mut remaining = width.saturating_sub(1);
    for span in &line.spans {
        if remaining == 0 {
            break;
        }

        let text = take_terminal_chars(span.content.as_ref(), remaining);
        if text.is_empty() {
            continue;
        }
        remaining = remaining.saturating_sub(UnicodeWidthStr::width(text.as_str()));
        apply_terminal_style(terminal, span.style)?;
        execute!(terminal.backend_mut(), Print(text))?;
    }

    execute!(
        terminal.backend_mut(),
        ResetColor,
        SetAttribute(Attribute::Reset),
        Print("\r\n")
    )?;
    Ok(())
}

fn truncate_terminal_line(line: &str, width: usize) -> String {
    take_terminal_chars(line, width.saturating_sub(1))
}

fn take_terminal_chars(line: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in line.chars().take(width) {
        let ch_width = ch.width().unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out
}

fn apply_terminal_style(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    style: Style,
) -> Result<()> {
    execute!(
        terminal.backend_mut(),
        ResetColor,
        SetAttribute(Attribute::Reset)
    )?;
    if let Some(color) = style.fg.and_then(to_crossterm_color) {
        execute!(terminal.backend_mut(), SetForegroundColor(color))?;
    }
    if style.add_modifier.contains(Modifier::BOLD) {
        execute!(terminal.backend_mut(), SetAttribute(Attribute::Bold))?;
    }
    if style.add_modifier.contains(Modifier::ITALIC) {
        execute!(terminal.backend_mut(), SetAttribute(Attribute::Italic))?;
    }
    if style.add_modifier.contains(Modifier::DIM) {
        execute!(terminal.backend_mut(), SetAttribute(Attribute::Dim))?;
    }
    Ok(())
}

fn to_crossterm_color(color: RColor) -> Option<CTermColor> {
    match color {
        RColor::Reset => None,
        RColor::Black => Some(CTermColor::Black),
        RColor::Red => Some(CTermColor::DarkRed),
        RColor::Green => Some(CTermColor::DarkGreen),
        RColor::Yellow => Some(CTermColor::DarkYellow),
        RColor::Blue => Some(CTermColor::DarkBlue),
        RColor::Magenta => Some(CTermColor::DarkMagenta),
        RColor::Cyan => Some(CTermColor::DarkCyan),
        RColor::Gray => Some(CTermColor::Grey),
        RColor::DarkGray => Some(CTermColor::DarkGrey),
        RColor::LightRed => Some(CTermColor::Red),
        RColor::LightGreen => Some(CTermColor::Green),
        RColor::LightYellow => Some(CTermColor::Yellow),
        RColor::LightBlue => Some(CTermColor::Blue),
        RColor::LightMagenta => Some(CTermColor::Magenta),
        RColor::LightCyan => Some(CTermColor::Cyan),
        RColor::White => Some(CTermColor::White),
        RColor::Rgb(r, g, b) => Some(CTermColor::Rgb { r, g, b }),
        RColor::Indexed(index) => Some(CTermColor::AnsiValue(index)),
    }
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
                "/help commands: /clear, /cancel, /resume [session-dir], /session, /context, /quit",
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
        "/context" => {
            state.push_system(state.context_report());
        }
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
    let mut resumed_config = driver_config.clone();
    resumed_config.resume_session = Some(session_dir.clone());

    driver.shutdown().await?;
    *driver = AgentDriver::spawn(resumed_config).await?;
    state.reset_after_resume_with_history(session_dir, history);
    Ok(())
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
    }
    if let Some(default_path) = default_config_path() {
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
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
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
    fn handled_key_events_include_repeat_for_wheel_scroll() {
        assert!(is_handled_key_event(KeyEventKind::Press));
        assert!(is_handled_key_event(KeyEventKind::Repeat));
        assert!(!is_handled_key_event(KeyEventKind::Release));
    }
}
