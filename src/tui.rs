use std::{
    io::{self, Stdout},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use modular_agent::{
    core::{AgentRuntime, AppConfig},
    domain::AgentOutput,
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::Value;
use tokio::task::JoinHandle;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub async fn run_tui(runtime: AgentRuntime, config: AppConfig, cwd: PathBuf) -> Result<()> {
    let mut terminal = TerminalSession::enter()?;
    let runtime = Arc::new(runtime);
    let mut app = TuiApp::new(
        config,
        cwd,
        runtime.session_dir().map(|path| path.to_path_buf()),
    )?;

    let mut dirty = true;
    loop {
        if dirty {
            terminal.draw(|frame| render(frame, &app))?;
            dirty = false;
        }

        if app.should_quit {
            break;
        }

        let spinner_before = app.spinner_index;
        if let Some(output) = app.poll_completed_model().await? {
            app.start_streaming(output)?;
            dirty = true;
            continue;
        }
        if app.spinner_index != spinner_before {
            dirty = true;
        }

        if app.advance_streaming() {
            dirty = true;
        }

        let poll_delay = if app.is_busy() {
            Duration::from_millis(25)
        } else {
            Duration::from_millis(250)
        };
        if event::poll(poll_delay)? {
            match event::read()? {
                CrosstermEvent::Key(key) => {
                    dirty |= handle_key(key, &runtime, &mut app).await?;
                }
                CrosstermEvent::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        app.scroll_up(3);
                        dirty = true;
                    }
                    MouseEventKind::ScrollDown => {
                        app.scroll_down(3);
                        dirty = true;
                    }
                    _ => {}
                },
                CrosstermEvent::Resize(_, _) => dirty = true,
                _ => {}
            }
        }
    }

    terminal.show_cursor()?;
    Ok(())
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw terminal mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("failed to create terminal")?;
        Ok(Self { terminal })
    }

    fn draw<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut Frame),
    {
        self.terminal.draw(f)?;
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<()> {
        self.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        );
    }
}

struct TuiApp {
    config: AppConfig,
    cwd: PathBuf,
    session_dir: Option<PathBuf>,
    header_model: String,
    messages: Vec<TuiMessage>,
    input: String,
    footer: String,
    status: String,
    scroll_offset: usize,
    pending: Option<JoinHandle<Result<AgentOutput>>>,
    spinner_index: usize,
    last_tick: Instant,
    streaming: Option<StreamingAnswer>,
    should_quit: bool,
}

impl TuiApp {
    fn new(config: AppConfig, cwd: PathBuf, session_dir: Option<PathBuf>) -> Result<Self> {
        let model = config.active_model_config()?;
        let header_model = format!("{}/{}", model.provider, model.model);
        Ok(Self {
            footer: format!("? for shortcuts  ·  model {header_model}  ·  Context waiting"),
            status: "ready".to_owned(),
            config,
            cwd,
            session_dir,
            header_model,
            messages: vec![TuiMessage::system("Welcome back. Type /help or /exit.")],
            input: String::new(),
            scroll_offset: 0,
            pending: None,
            spinner_index: 0,
            last_tick: Instant::now(),
            streaming: None,
            should_quit: false,
        })
    }

    async fn poll_completed_model(&mut self) -> Result<Option<AgentOutput>> {
        let Some(handle) = self.pending.as_ref() else {
            return Ok(None);
        };
        if !handle.is_finished() {
            if self.last_tick.elapsed() >= Duration::from_millis(120) {
                self.spinner_index = self.spinner_index.wrapping_add(1);
                self.last_tick = Instant::now();
            }
            return Ok(None);
        }

        let handle = self.pending.take().expect("pending handle exists");
        self.status = "rendering".to_owned();
        match handle.await {
            Ok(Ok(output)) => Ok(Some(output)),
            Ok(Err(error)) => {
                self.messages
                    .push(TuiMessage::error(format!("error: {error:#}")));
                self.status = "error".to_owned();
                Ok(None)
            }
            Err(error) => {
                self.messages
                    .push(TuiMessage::error(format!("task join error: {error:#}")));
                self.status = "error".to_owned();
                Ok(None)
            }
        }
    }

    fn start_streaming(&mut self, output: AgentOutput) -> Result<()> {
        self.footer = footer_from_output(&self.config, &output)?;
        self.messages.push(TuiMessage::assistant(String::new()));
        self.scroll_to_bottom();
        self.streaming = Some(StreamingAnswer {
            full_text: output.text,
            shown: 0,
            last_tick: Instant::now(),
        });
        self.status = "streaming".to_owned();
        Ok(())
    }

    fn advance_streaming(&mut self) -> bool {
        let Some(streaming) = &mut self.streaming else {
            return false;
        };
        if streaming.last_tick.elapsed() < Duration::from_millis(16) {
            return false;
        }

        let total = streaming.full_text.chars().count();
        let remaining = total.saturating_sub(streaming.shown);
        let batch = if total > 2_000 {
            48
        } else if total > 800 {
            24
        } else {
            8
        };
        streaming.shown += remaining.min(batch);
        streaming.last_tick = Instant::now();

        if let Some(last) = self.messages.last_mut() {
            last.text = streaming
                .full_text
                .chars()
                .take(streaming.shown)
                .collect::<String>();
        }

        if streaming.shown >= total {
            self.streaming = None;
            self.status = "ready".to_owned();
        }
        true
    }

    fn is_busy(&self) -> bool {
        self.pending.is_some() || self.streaming.is_some()
    }

    fn scroll_up(&mut self, rows: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(rows);
    }

    fn scroll_down(&mut self, rows: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(rows);
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }
}

struct StreamingAnswer {
    full_text: String,
    shown: usize,
    last_tick: Instant,
}

struct TuiMessage {
    role: TuiRole,
    text: String,
}

impl TuiMessage {
    fn user(text: impl Into<String>) -> Self {
        Self {
            role: TuiRole::User,
            text: text.into(),
        }
    }

    fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: TuiRole::Assistant,
            text: text.into(),
        }
    }

    fn system(text: impl Into<String>) -> Self {
        Self {
            role: TuiRole::System,
            text: text.into(),
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            role: TuiRole::Error,
            text: text.into(),
        }
    }
}

enum TuiRole {
    User,
    Assistant,
    System,
    Error,
}

async fn handle_key(key: KeyEvent, runtime: &Arc<AgentRuntime>, app: &mut TuiApp) -> Result<bool> {
    if key.kind != KeyEventKind::Press {
        return Ok(false);
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                app.should_quit = true;
                return Ok(true);
            }
            KeyCode::Char('j') | KeyCode::Char('m') => {
                if !app.is_busy() {
                    submit_input(runtime, app).await?;
                    return Ok(true);
                }
                return Ok(false);
            }
            KeyCode::Char('u') => {
                app.scroll_up(8);
                return Ok(true);
            }
            KeyCode::Char('d') => {
                app.scroll_down(8);
                return Ok(true);
            }
            _ => return Ok(false),
        }
    }

    match key.code {
        KeyCode::PageUp => {
            app.scroll_up(8);
            return Ok(true);
        }
        KeyCode::PageDown => {
            app.scroll_down(8);
            return Ok(true);
        }
        KeyCode::Home => {
            app.scroll_up(usize::MAX / 4);
            return Ok(true);
        }
        KeyCode::End => {
            app.scroll_to_bottom();
            return Ok(true);
        }
        _ => {}
    }

    if app.is_busy() {
        return Ok(false);
    }

    let mut dirty = true;
    match key.code {
        KeyCode::Esc => app.should_quit = true,
        KeyCode::Char(ch) => app.input.push(ch),
        KeyCode::Backspace => {
            app.input.pop();
        }
        KeyCode::Enter => submit_input(runtime, app).await?,
        _ => dirty = false,
    }

    Ok(dirty)
}

async fn submit_input(runtime: &Arc<AgentRuntime>, app: &mut TuiApp) -> Result<()> {
    let input = app.input.trim().to_owned();
    app.input.clear();
    if input.is_empty() {
        return Ok(());
    }

    match input.as_str() {
        "/exit" | "/quit" => {
            app.should_quit = true;
            return Ok(());
        }
        "/clear" | "/reset" => {
            runtime.clear_history().await?;
            app.messages.clear();
            app.messages.push(TuiMessage::system("history cleared"));
            app.status = "ready".to_owned();
            return Ok(());
        }
        "/history" => {
            app.messages.push(TuiMessage::system(format!(
                "history messages: {}",
                runtime.history_len().await
            )));
            return Ok(());
        }
        "/help" => {
            app.messages.push(TuiMessage::system(
                "/help, /history, /clear, /reset, /exit. Type a task and press Enter.",
            ));
            return Ok(());
        }
        _ => {}
    }

    app.messages.push(TuiMessage::user(input.clone()));
    app.scroll_to_bottom();
    app.status = "thinking".to_owned();
    app.spinner_index = 0;
    app.last_tick = Instant::now();
    let runtime = Arc::clone(runtime);
    app.pending = Some(tokio::spawn(async move { runtime.run(input).await }));
    Ok(())
}

fn render(frame: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_transcript(frame, app, chunks[0]);
    render_composer(frame, app, chunks[1]);
    render_footer(frame, app, chunks[2]);

    let cursor_x = chunks[1].x + 3 + app.input.chars().count() as u16;
    let cursor_y = chunks[1].y + 1;
    frame.set_cursor_position(Position::new(cursor_x.min(chunks[1].right() - 2), cursor_y));
}

fn render_transcript(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let mut lines = startup_card(app);
    for message in &app.messages {
        append_message_lines(&mut lines, message, area.width as usize);
        lines.push(Line::raw(""));
    }

    let height = area.height as usize;
    let max_offset = lines.len().saturating_sub(height);
    let offset = app.scroll_offset.min(max_offset);
    let end = lines.len().saturating_sub(offset);
    let start = end.saturating_sub(height);
    let visible = if lines.len() > height {
        lines[start..end].to_vec()
    } else {
        lines
    };

    frame.render_widget(Paragraph::new(visible), area);
}

fn startup_card(app: &TuiApp) -> Vec<Line<'static>> {
    let model = format!("model:     {}", app.header_model);
    let directory = format!("directory: {}", app.cwd.display());
    let session = app
        .session_dir
        .as_ref()
        .map(|path| format!("session:   {}", path.display()))
        .unwrap_or_else(|| "session:   not persisted".to_owned());
    let width = [
        model.chars().count(),
        directory.chars().count(),
        session.chars().count(),
        30,
    ]
    .into_iter()
    .max()
    .unwrap_or(30);
    let title = ">_ Modular Agent";
    let right = width.saturating_sub(title.chars().count());
    vec![
        Line::from(Span::styled(
            format!("╭─{}{}╮", title, "─".repeat(right)),
            Style::default().fg(Color::DarkGray),
        )),
        card_line(&model, width),
        card_line(&directory, width),
        card_line(&session, width),
        Line::from(Span::styled(
            format!("╰{}╯", "─".repeat(width + 2)),
            Style::default().fg(Color::DarkGray),
        )),
        Line::raw(""),
    ]
}

fn card_line(text: &str, width: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled("│ ", Style::default().fg(Color::DarkGray)),
        Span::raw(text.to_owned()),
        Span::raw(" ".repeat(width.saturating_sub(text.chars().count()))),
        Span::styled(" │", Style::default().fg(Color::DarkGray)),
    ])
}

fn append_message_lines(lines: &mut Vec<Line<'static>>, message: &TuiMessage, width: usize) {
    let (prefix, style) = match message.role {
        TuiRole::User => ("› ", Style::default().fg(Color::Cyan)),
        TuiRole::Assistant => ("• ", Style::default().fg(Color::Reset)),
        TuiRole::System => ("  ", Style::default().fg(Color::DarkGray)),
        TuiRole::Error => ("! ", Style::default().fg(Color::Red)),
    };

    let text_width = width.saturating_sub(prefix.chars().count()).max(1);
    if message.text.is_empty() {
        lines.push(Line::from(Span::styled(
            prefix.trim_end().to_owned(),
            style,
        )));
        return;
    }

    let mut first_segment = true;
    for source_line in message.text.lines() {
        let segments = wrap_text(source_line, text_width);
        if segments.is_empty() {
            lines.push(Line::raw(""));
            continue;
        }

        for segment in segments {
            let line_prefix = if first_segment { prefix } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(line_prefix.to_owned(), style),
                Span::styled(segment, style),
            ]));
            first_segment = false;
        }
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut segments = Vec::new();
    let mut segment = String::new();
    for ch in text.chars() {
        segment.push(ch);
        if segment.chars().count() >= width {
            segments.push(std::mem::take(&mut segment));
        }
    }
    if !segment.is_empty() {
        segments.push(segment);
    }
    segments
}

fn render_composer(frame: &mut Frame, app: &TuiApp, area: ratatui::layout::Rect) {
    let status = if app.pending.is_some() {
        format!("{} thinking", SPINNER[app.spinner_index % SPINNER.len()])
    } else {
        app.status.clone()
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("❯ ", Style::default().fg(Color::Cyan)),
            Span::raw(app.input.clone()),
        ]),
        Line::from(Span::styled(status, Style::default().fg(Color::DarkGray))),
    ];
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_footer(frame: &mut Frame, app: &TuiApp, area: ratatui::layout::Rect) {
    let footer = Paragraph::new(Line::from(Span::styled(
        app.footer.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(footer, area);
}

fn footer_from_output(config: &AppConfig, output: &AgentOutput) -> Result<String> {
    let model = footer_model(config, output)?;
    let context = footer_context(config, output);
    let session = output
        .metadata
        .get("session_id")
        .and_then(Value::as_str)
        .map(short_id)
        .unwrap_or("unknown");
    Ok(format!(
        "? for shortcuts  ·  {model}  ·  {context}  ·  session {session}"
    ))
}

fn footer_model(config: &AppConfig, output: &AgentOutput) -> Result<String> {
    if let Some(model) = output.metadata.get("model") {
        let provider = model.get("provider").and_then(Value::as_str);
        let name = model
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| model.get("model").and_then(Value::as_str));
        if let Some(name) = name {
            return Ok(match provider {
                Some(provider) if !provider.is_empty() => format!("model {provider}/{name}"),
                _ => format!("model {name}"),
            });
        }
    }

    let model = config.active_model_config()?;
    Ok(format!("model {}/{}", model.provider, model.model))
}

fn footer_context(config: &AppConfig, output: &AgentOutput) -> String {
    let context = output.metadata.get("context");
    let tokens = context
        .and_then(|context| context.get("token_estimate"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let max_tokens = config
        .renderer
        .statusline
        .context
        .max_tokens
        .unwrap_or(200_000)
        .max(1);
    let percent = ((tokens as f64 / max_tokens as f64) * 100.0).clamp(0.0, 100.0);
    format!("Context {:.0}% · {} in", percent, tokens)
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
