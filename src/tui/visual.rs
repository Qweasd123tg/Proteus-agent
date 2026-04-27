use std::path::{Path, PathBuf};

use modular_agent::contracts::ApprovalRequest;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(crate) struct VisualState<'a> {
    pub model: &'a str,
    pub cwd: &'a Path,
    pub session_dir: Option<&'a Path>,
    pub messages: &'a [VisualMessage],
    pub input: &'a str,
    pub footer: &'a str,
    pub status: &'a str,
    pub spinner_index: usize,
    pub scroll_offset: usize,
    pub pending_approval: Option<&'a ApprovalRequest>,
    pub pending_model: bool,
    pub streaming: bool,
}

pub(crate) struct VisualSurface {
    transcript: TranscriptComponent,
    composer: ComposerComponent,
    footer: FooterComponent,
    approval: ApprovalComponent,
}

impl Default for VisualSurface {
    fn default() -> Self {
        Self {
            transcript: TranscriptComponent,
            composer: ComposerComponent,
            footer: FooterComponent,
            approval: ApprovalComponent,
        }
    }
}

impl VisualSurface {
    pub(crate) fn render(&self, frame: &mut Frame, state: &VisualState<'_>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),
                Constraint::Length(2),
                Constraint::Length(1),
            ])
            .split(frame.area());

        self.transcript.render(frame, chunks[0], state);
        self.composer.render(frame, chunks[1], state);
        self.footer.render(frame, chunks[2], state);

        let cursor_x = chunks[1].x + 2 + state.input.chars().count() as u16;
        let cursor_y = chunks[1].y;
        frame.set_cursor_position(Position::new(
            cursor_x.min(chunks[1].right().saturating_sub(1)),
            cursor_y,
        ));

        if let Some(request) = state.pending_approval {
            self.approval.render(frame, frame.area(), request);
        }
    }
}

trait VisualComponent {
    fn render(&self, frame: &mut Frame, area: Rect, state: &VisualState<'_>);
}

struct TranscriptComponent;
struct ComposerComponent;
struct FooterComponent;
struct ApprovalComponent;

impl VisualComponent for TranscriptComponent {
    fn render(&self, frame: &mut Frame, area: Rect, state: &VisualState<'_>) {
        let mut lines = session_card(state, area.width as usize);
        for message in state.messages {
            append_message_lines(&mut lines, message, area.width as usize);
            lines.push(Line::raw(""));
        }
        if state.pending_model && !state.streaming {
            lines.push(Line::from(vec![
                Span::styled("• ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!(
                        "{} Working (esc to interrupt)",
                        SPINNER[state.spinner_index % SPINNER.len()]
                    ),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
            lines.push(Line::raw(""));
        }
        if state.scroll_offset > 0 {
            lines.push(Line::from(Span::styled(
                format!("  ↑ {} lines above bottom", state.scroll_offset),
                Style::default().fg(Color::DarkGray),
            )));
        }

        let height = area.height as usize;
        let max_offset = lines.len().saturating_sub(height);
        let offset = state.scroll_offset.min(max_offset);
        let end = lines.len().saturating_sub(offset);
        let start = end.saturating_sub(height);
        let visible = if lines.len() > height {
            lines[start..end].to_vec()
        } else {
            lines
        };

        frame.render_widget(Paragraph::new(visible), area);
    }
}

impl VisualComponent for ComposerComponent {
    fn render(&self, frame: &mut Frame, area: Rect, state: &VisualState<'_>) {
        let input = if state.input.is_empty() && !state.pending_model {
            Span::styled(
                "Ask agent to do anything",
                Style::default().fg(Color::DarkGray),
            )
        } else {
            Span::raw(state.input.to_owned())
        };
        let prompt = if state.pending_approval.is_some() {
            Span::styled("?", Style::default().fg(Color::Yellow))
        } else {
            Span::styled("›", Style::default().fg(Color::Cyan))
        };
        let lines = vec![
            Line::from(vec![prompt, Span::raw(" "), input]),
            Line::raw(""),
        ];
        frame.render_widget(Paragraph::new(lines), area);
    }
}

impl VisualComponent for FooterComponent {
    fn render(&self, frame: &mut Frame, area: Rect, state: &VisualState<'_>) {
        let left = if state.pending_approval.is_some() {
            "y approve · n deny · esc deny".to_owned()
        } else if state.scroll_offset > 0 {
            "end to bottom · page up/down scroll".to_owned()
        } else {
            state.status.to_owned()
        };
        let line = truncate(format!("  {left}    {}", state.footer), area.width as usize);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                line,
                Style::default().fg(Color::DarkGray),
            ))),
            area,
        );
    }
}

impl ApprovalComponent {
    fn render(&self, frame: &mut Frame, full: Rect, request: &ApprovalRequest) {
        let modal_width = full
            .width
            .saturating_mul(3)
            .saturating_div(4)
            .clamp(50, 96)
            .min(full.width.saturating_sub(2));
        let modal_height = 11u16.min(full.height.saturating_sub(2));
        let x = full.x + full.width.saturating_sub(modal_width) / 2;
        let y = full.y + full.height.saturating_sub(modal_height) / 2;
        let area = Rect::new(x, y, modal_width, modal_height);

        frame.render_widget(Clear, area);

        let safety = request
            .tool_spec
            .as_ref()
            .map(|spec| format!("{:?}", spec.safety))
            .unwrap_or_else(|| "unknown".to_owned());
        let args = compact_value(&request.call.args);
        let inner_width = area.width.saturating_sub(4) as usize;

        let mut body: Vec<Line<'static>> = Vec::new();
        body.push(Line::from(vec![
            Span::styled("tool ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                request.call.name.clone(),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(format!(" · {safety}"), Style::default().fg(Color::DarkGray)),
        ]));
        body.push(Line::from(format!("cwd  {}", request.cwd.display())));
        body.push(Line::raw(""));
        for seg in wrap_text(&request.reason, inner_width) {
            body.push(Line::from(seg));
        }
        for seg in wrap_text(&format!("args {args}"), inner_width) {
            body.push(Line::from(Span::styled(
                seg,
                Style::default().fg(Color::DarkGray),
            )));
        }
        body.push(Line::raw(""));
        body.push(Line::from(vec![
            Span::styled("y", Style::default().fg(Color::Green)),
            Span::raw(" approve    "),
            Span::styled("n / esc", Style::default().fg(Color::Red)),
            Span::raw(" deny"),
        ]));

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" approval ");
        frame.render_widget(Paragraph::new(body).block(block), area);
    }
}

#[derive(Clone)]
pub(crate) struct VisualMessage {
    pub role: VisualRole,
    pub text: String,
    pub tool: Option<ToolCard>,
}

impl VisualMessage {
    pub(crate) fn user(text: impl Into<String>) -> Self {
        Self {
            role: VisualRole::User,
            text: text.into(),
            tool: None,
        }
    }

    pub(crate) fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: VisualRole::Assistant,
            text: text.into(),
            tool: None,
        }
    }

    pub(crate) fn system(text: impl Into<String>) -> Self {
        Self {
            role: VisualRole::System,
            text: text.into(),
            tool: None,
        }
    }

    pub(crate) fn error(text: impl Into<String>) -> Self {
        Self {
            role: VisualRole::Error,
            text: text.into(),
            tool: None,
        }
    }

    pub(crate) fn tool(card: ToolCard) -> Self {
        Self {
            role: VisualRole::Tool,
            text: String::new(),
            tool: Some(card),
        }
    }
}

#[derive(Clone)]
pub(crate) enum VisualRole {
    User,
    Assistant,
    System,
    Error,
    Tool,
}

#[derive(Clone)]
pub(crate) struct ToolCard {
    pub call_id: modular_agent::domain::CallId,
    pub name: String,
    pub args_summary: String,
    pub status: ToolStatus,
    pub output_preview: String,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ToolStatus {
    Running,
    Ok,
    Err,
}

fn session_card(state: &VisualState<'_>, width: usize) -> Vec<Line<'static>> {
    let cwd = display_path(state.cwd);
    let session = state
        .session_dir
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(short_id)
        .unwrap_or("not persisted")
        .to_owned();
    let rows = [
        format!("model:     {}", state.model),
        format!("directory: {cwd}"),
        format!("session:   {session}"),
    ];
    let content_width = rows
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(30)
        .max(30)
        .min(width.saturating_sub(4).max(24));
    let title = ">_ Modular Agent";
    let right = content_width.saturating_sub(title.chars().count());

    let mut lines = vec![Line::from(Span::styled(
        format!("╭─{}{}╮", title, "─".repeat(right)),
        Style::default().fg(Color::DarkGray),
    ))];
    for row in rows {
        lines.push(card_line(&truncate(row, content_width), content_width));
    }
    lines.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(content_width + 2)),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::raw(""));
    lines
}

fn card_line(text: &str, width: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled("│ ", Style::default().fg(Color::DarkGray)),
        Span::raw(text.to_owned()),
        Span::raw(" ".repeat(width.saturating_sub(text.chars().count()))),
        Span::styled(" │", Style::default().fg(Color::DarkGray)),
    ])
}

fn append_message_lines(lines: &mut Vec<Line<'static>>, message: &VisualMessage, width: usize) {
    if matches!(message.role, VisualRole::Tool) {
        if let Some(card) = &message.tool {
            append_tool_card_lines(lines, card, width);
        }
        return;
    }

    let (prefix, style) = match message.role {
        VisualRole::User => ("› ", Style::default().fg(Color::Cyan)),
        VisualRole::Assistant => ("• ", Style::default().fg(Color::Reset)),
        VisualRole::System => ("  ", Style::default().fg(Color::DarkGray)),
        VisualRole::Error => ("! ", Style::default().fg(Color::Red)),
        VisualRole::Tool => ("  ", Style::default().fg(Color::DarkGray)),
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

fn append_tool_card_lines(lines: &mut Vec<Line<'static>>, card: &ToolCard, width: usize) {
    let (glyph, glyph_style, name_style) = match card.status {
        ToolStatus::Running => (
            "⠋",
            Style::default().fg(Color::Yellow),
            Style::default().fg(Color::Yellow),
        ),
        ToolStatus::Ok => (
            "✓",
            Style::default().fg(Color::Green),
            Style::default().fg(Color::Reset),
        ),
        ToolStatus::Err => (
            "✗",
            Style::default().fg(Color::Red),
            Style::default().fg(Color::Red),
        ),
    };
    let mut header: Vec<Span<'static>> = vec![
        Span::styled(format!("{glyph} "), glyph_style),
        Span::styled(card.name.clone(), name_style),
    ];
    if !card.args_summary.is_empty() {
        header.push(Span::styled(
            format!(" · {}", card.args_summary),
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::from(header));

    if !card.output_preview.is_empty() {
        let preview_width = width.saturating_sub(4).max(1);
        for raw in card.output_preview.lines() {
            for segment in wrap_text(raw, preview_width) {
                lines.push(Line::from(vec![
                    Span::styled("  ↳ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(segment, Style::default().fg(Color::DarkGray)),
                ]));
            }
        }
    }
}

pub(crate) fn wrap_text(text: &str, width: usize) -> Vec<String> {
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

pub(crate) fn compact_value(value: &serde_json::Value) -> String {
    let rendered = match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let collapsed: String = rendered
        .chars()
        .map(|ch| if ch == '\n' || ch == '\r' { ' ' } else { ch })
        .collect();
    truncate(collapsed, 80)
}

pub(crate) fn truncate(input: impl Into<String>, width: usize) -> String {
    let input = input.into();
    if input.chars().count() <= width {
        return input;
    }
    if width <= 1 {
        return "…".to_owned();
    }
    let prefix: String = input.chars().take(width - 1).collect();
    format!("{prefix}…")
}

fn display_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(&home)
    {
        if rest.as_os_str().is_empty() {
            return "~".to_owned();
        }
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
