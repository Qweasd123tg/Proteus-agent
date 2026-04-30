//! Experimental client: Codex TUI style.
//!
//! Отличия от `agent-tui`:
//! - Inline viewport вместо alternate screen. Транскрипт идёт в обычный
//!   scrollback терминала — листай мышью, грепни в истории shell, копируй
//!   как обычный текст.
//! - `insert_before` вставляет каждое сообщение/tool-card НАД viewport'ом,
//!   оно уходит в scrollback и остаётся доступным.
//! - Bottom composer с рамкой — единственное "живое" UI.
//! - Bracketed paste enabled, mouse capture — нет.

use std::{io, path::PathBuf, time::Duration};

use agent_contracts::app_protocol::{StdioOutput, StdioRequest};
use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event as CTerm, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

#[path = "../driver.rs"]
mod driver;

use driver::{AgentDriver, DriverConfig};

const VIEWPORT_ROWS: u16 = 5;

// --- state ---------------------------------------------------------------

struct AppState {
    input: String,
    status: String,
    model_label: String,
    spinner_index: usize,
    pending_model: bool,
    pending_approval: Option<PendingApproval>,
    should_quit: bool,
    #[allow(dead_code)]
    cwd: PathBuf,
}

struct PendingApproval {
    approval_id: String,
    tool_name: String,
    reason: String,
}

impl AppState {
    fn new(cwd: PathBuf) -> Self {
        Self {
            input: String::new(),
            status: "ready".into(),
            model_label: "unknown".into(),
            spinner_index: 0,
            pending_model: false,
            pending_approval: None,
            should_quit: false,
            cwd,
        }
    }
}

// --- main ----------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let cli = parse_args()?;
    install_panic_hook();

    enable_raw_mode()?;
    execute!(io::stdout(), EnableBracketedPaste)?;

    let result = run_app(cli).await;

    execute!(io::stdout(), DisableBracketedPaste).ok();
    disable_raw_mode().ok();
    println!();

    if let Err(ref err) = result {
        eprintln!("agent-tui-codex: {err:#}");
    }
    result
}

struct Cli {
    agent_bin: Option<PathBuf>,
    config_path: Option<PathBuf>,
    cwd: Option<PathBuf>,
}

fn parse_args() -> Result<Cli> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut agent_bin = None;
    let mut config_path = None;
    let mut cwd = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--agent-bin" => agent_bin = it.next().map(PathBuf::from),
            "--config" => config_path = it.next().map(PathBuf::from),
            "--cwd" => cwd = it.next().map(PathBuf::from),
            "-h" | "--help" => {
                eprintln!(
                    "agent-tui-codex — experimental Codex-style TUI\n\nusage: agent-tui-codex [--agent-bin PATH] [--config PATH] [--cwd PATH]"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown arg: {other}");
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

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        let _ = disable_raw_mode();
        println!();
        let path = std::env::temp_dir().join("agent-tui-codex-panic.log");
        let _ = std::fs::write(&path, format!("{info}\n"));
        eprintln!("panic log: {}", path.display());
        prev(info);
    }));
}

async fn run_app(cli: Cli) -> Result<()> {
    let cwd = cli
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Inline(VIEWPORT_ROWS),
        },
    )
    .context("failed to init inline terminal")?;

    let mut driver = AgentDriver::spawn(DriverConfig {
        agent_bin: cli.agent_bin,
        config_path: cli.config_path,
        cwd: Some(cwd.clone()),
    })
    .await?;

    let mut state = AppState::new(cwd.clone());

    push_history(&mut terminal, session_header_lines(&state.model_label, &cwd))?;
    push_history(
        &mut terminal,
        vec![
            Line::from(Span::styled(
                "Connected to modular-agent. Type and press Enter.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::raw(""),
        ],
    )?;

    let (input_tx, mut input_rx) = tokio::sync::mpsc::channel::<CTerm>(64);
    std::thread::spawn(move || {
        loop {
            match event::read() {
                Ok(ev) => {
                    if input_tx.blocking_send(ev).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut tick = tokio::time::interval(Duration::from_millis(120));
    let mut dirty = true;

    loop {
        if dirty {
            draw_bottom_pane(&mut terminal, &state)?;
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
                        handle_app_event(&mut terminal, &mut state, *event)?;
                        dirty = true;
                    }
                    Some(StdioOutput::Response { ok, error, .. }) => {
                        if !ok {
                            push_history(&mut terminal, vec![
                                Line::from(vec![
                                    Span::styled("! ", Style::default().fg(Color::Red)),
                                    Span::raw(error.unwrap_or_else(|| "request failed".into())),
                                ]),
                            ])?;
                            dirty = true;
                        }
                    }
                    Some(_) => {}
                    None => {
                        push_history(&mut terminal, vec![
                            Line::from(Span::styled(
                                "agent process exited",
                                Style::default().fg(Color::Red),
                            )),
                        ])?;
                        state.should_quit = true;
                        dirty = true;
                    }
                }
            }

            term = input_rx.recv() => {
                if let Some(ev) = term
                    && handle_term_event(&mut terminal, &mut state, &mut driver, ev).await?
                {
                    dirty = true;
                }
            }

            _ = tick.tick() => {
                if state.pending_model || state.pending_approval.is_some() {
                    state.spinner_index = state.spinner_index.wrapping_add(1);
                    dirty = true;
                }
            }
        }
    }

    let _ = driver.shutdown().await;
    Ok(())
}

async fn handle_term_event(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    driver: &mut AgentDriver,
    ev: CTerm,
) -> Result<bool> {
    match ev {
        CTerm::Paste(text) => {
            state.input.push_str(&text);
            return Ok(true);
        }
        CTerm::Key(key) if key.kind == KeyEventKind::Press => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('c') => {
                        state.should_quit = true;
                        return Ok(true);
                    }
                    KeyCode::Char('l') => {
                        driver.send(&StdioRequest::ClearHistory { id: None }).await?;
                        push_history(
                            terminal,
                            vec![Line::from(Span::styled(
                                "-- history cleared --",
                                Style::default().fg(Color::DarkGray),
                            ))],
                        )?;
                        return Ok(true);
                    }
                    _ => {}
                }
            }

            if let Some(pending) = state.pending_approval.as_ref() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        let id = pending.approval_id.clone();
                        state.pending_approval = None;
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
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        let id = pending.approval_id.clone();
                        state.pending_approval = None;
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
                    _ => return Ok(false),
                }
            }

            match key.code {
                KeyCode::Enter => {
                    let text = state.input.trim().to_owned();
                    if text.is_empty() {
                        return Ok(false);
                    }
                    state.input.clear();
                    push_history(
                        terminal,
                        vec![
                            Line::from(vec![
                                Span::styled("› ", Style::default().fg(Color::Cyan)),
                                Span::styled(
                                    text.clone(),
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                            ]),
                            Line::raw(""),
                        ],
                    )?;
                    state.pending_model = true;
                    state.status = "thinking...".into();
                    driver.send(&StdioRequest::Send { id: None, text }).await?;
                    return Ok(true);
                }
                KeyCode::Backspace => {
                    state.input.pop();
                    return Ok(true);
                }
                KeyCode::Char(ch) => {
                    state.input.push(ch);
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

fn handle_app_event(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    event: agent_contracts::app_protocol::AppServerEvent,
) -> Result<()> {
    use agent_contracts::app_protocol::AppServerEvent as E;
    use agent_contracts::domain::Event;

    match event {
        E::Runtime { event } => match event {
            Event::ContextBuilt { chunks, token_estimate } => {
                state.status = match token_estimate {
                    Some(t) => format!("context: {chunks} chunks, ~{t}t"),
                    None => format!("context: {chunks} chunks"),
                };
            }
            Event::ModelRequestPrepared { model } => {
                state.model_label = format!("{}/{}", model.provider, model.model);
                state.status = "calling model...".into();
            }
            Event::ModelResponseReceived { finish_reason } => {
                state.status = format!("model: {finish_reason:?}");
            }
            Event::ToolCallRequested { call } => {
                let args = compact_json(&call.args, 80);
                push_history(
                    terminal,
                    vec![Line::from(vec![
                        Span::styled("⠋ ", Style::default().fg(Color::Yellow)),
                        Span::styled(
                            call.name.clone(),
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!(" · {args}"), Style::default().fg(Color::DarkGray)),
                    ])],
                )?;
                state.status = format!("tool: {}", call.name);
            }
            Event::ToolFinished { result } => {
                let glyph = if result.ok { "✓" } else { "✗" };
                let style = if result.ok {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                };
                let mut lines = vec![Line::from(vec![
                    Span::styled(format!("  {glyph} "), style),
                    Span::raw(
                        result
                            .error
                            .clone()
                            .unwrap_or_else(|| format!("{} bytes", result.output.len())),
                    ),
                ])];
                if result.ok && !result.output.is_empty() {
                    let preview = truncate(&result.output, 400);
                    for line in preview.lines().take(6) {
                        lines.push(Line::from(vec![
                            Span::raw("    "),
                            Span::styled(
                                line.to_owned(),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]));
                    }
                }
                lines.push(Line::raw(""));
                push_history(terminal, lines)?;
            }
            Event::TurnFinished { .. } => {
                state.status = "ready".into();
            }
            Event::MemoryWritten { kind } => {
                state.status = format!("memory: {kind}");
            }
            _ => {}
        },
        E::UserMessageSubmitted { .. } => {}
        E::TurnOutput { output } => {
            let mut lines = Vec::new();
            for line in output.text.lines() {
                lines.push(Line::from(vec![
                    Span::styled("• ", Style::default().fg(Color::Reset)),
                    Span::raw(line.to_owned()),
                ]));
            }
            lines.push(Line::raw(""));
            push_history(terminal, lines)?;
            state.pending_model = false;
            state.status = "ready".into();
        }
        E::ApprovalRequested { request } => {
            state.pending_approval = Some(PendingApproval {
                approval_id: request.approval_id.clone(),
                tool_name: request.call.name.clone(),
                reason: request.reason.clone(),
            });
            state.status = format!("approval: {}", request.call.name);
        }
        E::ApprovalResolved { .. } => {
            state.pending_approval = None;
            state.status = "thinking...".into();
        }
        E::Error { message } => {
            push_history(
                terminal,
                vec![Line::from(vec![
                    Span::styled("! ", Style::default().fg(Color::Red)),
                    Span::styled(message, Style::default().fg(Color::Red)),
                ])],
            )?;
            state.pending_model = false;
            state.status = "error".into();
        }
        E::Shutdown => state.should_quit = true,
        _ => {}
    }
    Ok(())
}

// --- rendering ----------------------------------------------------------

fn session_header_lines(model: &str, cwd: &std::path::Path) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled("╭─ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "modular-agent",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ─", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled("│ model: ", Style::default().fg(Color::DarkGray)),
            Span::raw(model.to_owned()),
        ]),
        Line::from(vec![
            Span::styled("│ cwd:   ", Style::default().fg(Color::DarkGray)),
            Span::raw(display_cwd(cwd)),
        ]),
        Line::from(Span::styled("╰─", Style::default().fg(Color::DarkGray))),
        Line::raw(""),
    ]
}

fn push_history(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    lines: Vec<Line<'static>>,
) -> Result<()> {
    let height = lines.len() as u16;
    if height == 0 {
        return Ok(());
    }
    terminal.insert_before(height, |buf| {
        let mut y = buf.area.y;
        for line in lines {
            if y >= buf.area.y + buf.area.height {
                break;
            }
            let rect = Rect::new(buf.area.x, y, buf.area.width, 1);
            Paragraph::new(line).render(rect, buf);
            y += 1;
        }
    })?;
    Ok(())
}

fn draw_bottom_pane(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &AppState,
) -> Result<()> {
    terminal.draw(|frame| {
        let area = frame.area();
        let border_color = if state.pending_approval.is_some() {
            Color::Yellow
        } else if state.pending_model {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(Line::from(vec![
                Span::raw(" "),
                Span::styled(status_glyph(state), Style::default().fg(Color::Yellow)),
                Span::raw(" "),
                Span::styled(state.status.clone(), Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
            ]));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let prompt_line = if let Some(pending) = &state.pending_approval {
            Line::from(vec![
                Span::styled(
                    "? approve ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
                Span::raw(pending.tool_name.clone()),
                Span::styled(
                    format!(" — {}", pending.reason),
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        } else {
            let prompt = if state.pending_model {
                Span::styled("› ", Style::default().fg(Color::DarkGray))
            } else {
                Span::styled("› ", Style::default().fg(Color::Cyan))
            };
            let content = if state.input.is_empty() && !state.pending_model {
                Span::styled(
                    "ask agent — enter to send",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::raw(state.input.clone())
            };
            Line::from(vec![prompt, content])
        };

        let hint = if state.pending_approval.is_some() {
            "y approve · n deny · esc deny · ctrl+c quit"
        } else {
            "enter send · ctrl+c quit · ctrl+l clear"
        };
        let hint_line = Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray)));

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(inner);
        frame.render_widget(Paragraph::new(prompt_line), chunks[0]);
        frame.render_widget(Paragraph::new(hint_line), chunks[2]);
    })?;
    Ok(())
}

fn status_glyph(state: &AppState) -> String {
    const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    if state.pending_model || state.pending_approval.is_some() {
        SPINNER[state.spinner_index % SPINNER.len()].to_owned()
    } else {
        "●".to_owned()
    }
}

fn display_cwd(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(&home)
    {
        if rest.as_os_str().is_empty() {
            return "~".into();
        }
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

fn compact_json(value: &serde_json::Value, max: usize) -> String {
    let s = match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let collapsed: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    truncate(&collapsed, max)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else if max <= 1 {
        "…".into()
    } else {
        let prefix: String = s.chars().take(max - 1).collect();
        format!("{prefix}…")
    }
}

#[allow(dead_code)]
fn _uses() {
    let _ = std::marker::PhantomData::<dyn Widget>;
}
