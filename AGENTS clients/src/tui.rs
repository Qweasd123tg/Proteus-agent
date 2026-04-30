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
    contracts::ApprovalResponse,
    core::{AgentRuntime, AppConfig, BroadcastEventSink},
    domain::{AgentOutput, Event, ToolCall, ToolResult},
    modules::PendingApproval,
};
use ratatui::{Frame, Terminal, backend::CrosstermBackend};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

#[path = "tui/visual.rs"]
mod visual;

use visual::{ToolCard, ToolStatus, VisualMessage, VisualRole, VisualState, VisualSurface};

pub async fn run_tui(
    runtime: AgentRuntime,
    config: AppConfig,
    cwd: PathBuf,
    events: Arc<BroadcastEventSink>,
    mut approvals: mpsc::Receiver<PendingApproval>,
) -> Result<()> {
    let mut terminal = TerminalSession::enter()?;
    let runtime = Arc::new(runtime);
    let mut event_rx = events.subscribe();
    let mut app = TuiApp::new(
        config,
        cwd,
        runtime.session_dir().map(|path| path.to_path_buf()),
    )?;

    let mut dirty = true;
    loop {
        if dirty {
            terminal.draw(|frame| app.render(frame))?;
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

        loop {
            match event_rx.try_recv() {
                Ok(event) => {
                    app.ingest_event(event);
                    dirty = true;
                }
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    app.messages.push(VisualMessage::system(format!(
                        "… dropped {n} live events (TUI fell behind, log still complete)"
                    )));
                    dirty = true;
                }
                Err(broadcast::error::TryRecvError::Closed) => break,
            }
        }

        if app.pending_approval.is_none()
            && let Ok(pending) = approvals.try_recv()
        {
            app.pending_approval = Some(pending);
            app.status = "approval required".to_owned();
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
    messages: Vec<VisualMessage>,
    input: String,
    footer: String,
    status: String,
    scroll_offset: usize,
    pending: Option<JoinHandle<Result<AgentOutput>>>,
    spinner_index: usize,
    last_tick: Instant,
    streaming: Option<StreamingAnswer>,
    pending_approval: Option<PendingApproval>,
    visual_surface: VisualSurface,
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
            messages: Vec::new(),
            input: String::new(),
            scroll_offset: 0,
            pending: None,
            spinner_index: 0,
            last_tick: Instant::now(),
            streaming: None,
            pending_approval: None,
            visual_surface: VisualSurface::default(),
            should_quit: false,
        })
    }

    fn render(&self, frame: &mut Frame) {
        let approval = self
            .pending_approval
            .as_ref()
            .map(|pending| &pending.request);
        let state = VisualState {
            model: &self.header_model,
            cwd: &self.cwd,
            session_dir: self.session_dir.as_deref(),
            messages: &self.messages,
            input: &self.input,
            footer: &self.footer,
            status: &self.status,
            spinner_index: self.spinner_index,
            scroll_offset: self.scroll_offset,
            pending_approval: approval,
            pending_model: self.pending.is_some(),
            streaming: self.streaming.is_some(),
        };
        self.visual_surface.render(frame, &state);
    }

    fn resolve_approval(&mut self, approved: bool) {
        let Some(pending) = self.pending_approval.take() else {
            return;
        };
        let PendingApproval { request, responder } = pending;
        let response = ApprovalResponse {
            approved,
            note: if approved {
                None
            } else {
                Some(format!("tool call denied by user: {}", request.reason))
            },
        };
        let tool_name = request.call.name.clone();
        let _ = responder.send(response);
        self.messages.push(VisualMessage::system(format!(
            "{} {} for {tool_name}",
            if approved { "✓" } else { "✗" },
            if approved { "approved" } else { "denied" }
        )));
        self.status = "ready".to_owned();
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
                    .push(VisualMessage::error(format!("error: {error:#}")));
                self.status = "error".to_owned();
                Ok(None)
            }
            Err(error) => {
                self.messages
                    .push(VisualMessage::error(format!("task join error: {error:#}")));
                self.status = "error".to_owned();
                Ok(None)
            }
        }
    }

    fn start_streaming(&mut self, output: AgentOutput) -> Result<()> {
        self.footer = footer_from_output(&self.config, &output)?;
        self.messages.push(VisualMessage::assistant(String::new()));
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

    fn ingest_event(&mut self, event: Event) {
        match event {
            Event::ToolCallRequested { call } => {
                self.messages.push(VisualMessage::tool(tool_card(&call)));
                self.status = format!("running {}", call.name);
            }
            Event::ToolFinished { result } => {
                self.update_tool_result(&result);
                self.status = if result.ok {
                    "ready".to_owned()
                } else {
                    "tool error".to_owned()
                };
            }
            Event::ApprovalRequested { call_id: _, reason } => {
                self.messages.push(VisualMessage::system(format!(
                    "approval requested: {reason}"
                )));
            }
            Event::ApprovalResolved {
                call_id: _,
                approved,
            } => {
                self.messages.push(VisualMessage::system(if approved {
                    "approval granted".to_owned()
                } else {
                    "approval denied".to_owned()
                }));
            }
            Event::Error { message } => {
                self.messages.push(VisualMessage::error(message));
            }
            Event::ContextBuilt {
                chunks,
                token_estimate,
            } => {
                self.status = match token_estimate {
                    Some(tokens) => format!("context · {chunks} chunks · ~{tokens} tok"),
                    None => format!("context · {chunks} chunks"),
                };
            }
            Event::ModelResponseReceived { .. }
            | Event::ModelRequestPrepared { .. }
            | Event::MemoryWritten { .. }
            | Event::PatchApplied { .. }
            | Event::SessionStarted { .. }
            | Event::TaskReceived { .. }
            | Event::TurnStarted { .. }
            | Event::TurnFinished { .. } => {}
        }
        self.scroll_to_bottom();
    }

    fn update_tool_result(&mut self, result: &ToolResult) {
        for message in self.messages.iter_mut().rev() {
            if let VisualMessage {
                role: VisualRole::Tool,
                tool: Some(card),
                ..
            } = message
                && card.call_id == result.call_id
                && card.status == ToolStatus::Running
            {
                card.status = if result.ok {
                    ToolStatus::Ok
                } else {
                    ToolStatus::Err
                };
                card.output_preview = preview_output(&result.output, result.error.as_deref());
                return;
            }
        }
    }
}

struct StreamingAnswer {
    full_text: String,
    shown: usize,
    last_tick: Instant,
}

fn tool_card(call: &ToolCall) -> ToolCard {
    ToolCard {
        call_id: call.id.clone(),
        name: call.name.clone(),
        args_summary: summarize_args(&call.args),
        status: ToolStatus::Running,
        output_preview: String::new(),
    }
}

fn summarize_args(args: &Value) -> String {
    match args {
        Value::Object(map) => {
            let mut pairs = Vec::with_capacity(map.len());
            for (key, value) in map.iter().take(3) {
                pairs.push(format!("{key}={}", compact_value(value)));
            }
            if map.len() > 3 {
                pairs.push("…".to_owned());
            }
            pairs.join(" ")
        }
        other => compact_value(other),
    }
}

fn compact_value(value: &Value) -> String {
    let rendered = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let collapsed: String = rendered
        .chars()
        .map(|ch| if ch == '\n' || ch == '\r' { ' ' } else { ch })
        .collect();
    if collapsed.chars().count() > 60 {
        let prefix: String = collapsed.chars().take(57).collect();
        format!("{prefix}…")
    } else {
        collapsed
    }
}

fn preview_output(output: &str, error: Option<&str>) -> String {
    let source = if output.trim().is_empty() {
        error.unwrap_or_default()
    } else {
        output
    };
    let mut lines: Vec<&str> = source.lines().take(6).collect();
    let extra = source.lines().count().saturating_sub(lines.len());
    if extra > 0 {
        lines.push("…");
    }
    lines.join("\n")
}

async fn handle_key(key: KeyEvent, runtime: &Arc<AgentRuntime>, app: &mut TuiApp) -> Result<bool> {
    if key.kind != KeyEventKind::Press {
        return Ok(false);
    }

    if app.pending_approval.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.resolve_approval(true);
                return Ok(true);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.resolve_approval(false);
                return Ok(true);
            }
            _ => return Ok(false),
        }
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
            app.messages.push(VisualMessage::system("history cleared"));
            app.status = "ready".to_owned();
            return Ok(());
        }
        "/history" => {
            app.messages.push(VisualMessage::system(format!(
                "history messages: {}",
                runtime.history_len().await
            )));
            return Ok(());
        }
        "/help" => {
            app.messages.push(VisualMessage::system(
                "/help, /history, /clear, /reset, /exit. Type a task and press Enter.",
            ));
            return Ok(());
        }
        _ => {}
    }

    app.messages.push(VisualMessage::user(input.clone()));
    app.scroll_to_bottom();
    app.status = "thinking".to_owned();
    app.spinner_index = 0;
    app.last_tick = Instant::now();
    let runtime = Arc::clone(runtime);
    app.pending = Some(tokio::spawn(async move { runtime.run(input).await }));
    Ok(())
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
