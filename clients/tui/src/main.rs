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
    collections::HashSet,
    io,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

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
    session_picker::ResumePickerItem,
    state::AppState,
    visual::VisualSurface,
};

/// DECSET 1007 — alternate scroll mode. Терминал сам переводит wheel
/// в клавиши Up/Down. Выделение текста мышью остаётся стандартным,
/// потому что мы НЕ включаем EnableMouseCapture.
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
    // ровно как в bash.
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

            // Если показан approval — обрабатываем y/n и те же клавиши в RU раскладке.
            if state.has_pending_approval() {
                match key.code {
                    KeyCode::Char('y')
                    | KeyCode::Char('Y')
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
                    KeyCode::Char('n')
                    | KeyCode::Char('N')
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
                "/help commands: /clear, /cancel, /resume [session-dir], /session, /quit",
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
    let mut resumed_config = driver_config.clone();
    resumed_config.resume_session = Some(session_dir.clone());

    driver.shutdown().await?;
    *driver = AgentDriver::spawn(resumed_config).await?;
    state.reset_after_resume(session_dir);
    Ok(())
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
        let modified = file_modified_time(&messages_path)
            .or_else(|| file_modified_time(&metadata_path))
            .or_else(|| file_modified_time(&session_dir));
        sessions.push(ResumePickerItem {
            session_dir,
            title,
            detail: format!("updated {}", format_time_ago(modified)),
            updated: modified,
        });
    }
    Ok(())
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
        assert_eq!(sessions[0].title, "1234567890");
        assert_eq!(sessions[0].session_dir, valid_session);
    }
}
