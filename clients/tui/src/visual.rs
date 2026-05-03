use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use agent_contracts::app_protocol::AppApprovalRequest;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    session_picker::ResumePicker,
    slash_commands::{SlashCommand, matching_slash_commands},
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
    pub pending_approval: Option<&'a AppApprovalRequest>,
    pub pending_model: bool,
    pub streaming: bool,
    pub thinking_elapsed: Option<Duration>,
    pub resume_picker: Option<&'a ResumePicker>,
    pub slash_selection: usize,
}

pub(crate) struct VisualSurface {
    transcript: TranscriptComponent,
    composer: ComposerComponent,
    footer: FooterComponent,
    resume_picker: ResumePickerComponent,
    slash: SlashComponent,
}

impl Default for VisualSurface {
    fn default() -> Self {
        Self {
            transcript: TranscriptComponent,
            composer: ComposerComponent,
            footer: FooterComponent,
            resume_picker: ResumePickerComponent,
            slash: SlashComponent,
        }
    }
}

impl VisualSurface {
    pub(crate) fn render(&self, frame: &mut Frame, state: &VisualState<'_>) {
        if let Some(picker) = state.resume_picker {
            self.resume_picker.render(frame, frame.area(), picker);
            frame.set_cursor_position(Position::new(
                picker
                    .query
                    .chars()
                    .count()
                    .min(frame.area().width.saturating_sub(1) as usize) as u16,
                1,
            ));
            return;
        }

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
        self.slash.render(frame, chunks[1], state);

        let cursor_x = chunks[1].x + 2 + state.input.chars().count() as u16;
        let cursor_y = chunks[1].y;
        frame.set_cursor_position(Position::new(
            cursor_x.min(chunks[1].right().saturating_sub(1)),
            cursor_y,
        ));
    }
}

trait VisualComponent {
    fn render(&self, frame: &mut Frame, area: Rect, state: &VisualState<'_>);
}

struct TranscriptComponent;
struct ComposerComponent;
struct FooterComponent;
struct ResumePickerComponent;
struct SlashComponent;

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
                        "{} Working {}(esc to interrupt)",
                        SPINNER[state.spinner_index % SPINNER.len()],
                        state
                            .thinking_elapsed
                            .map(|elapsed| format!("{} ", format_elapsed(elapsed)))
                            .unwrap_or_default()
                    ),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
            lines.push(Line::raw(""));
        }
        if let Some(request) = state.pending_approval {
            append_approval_lines(&mut lines, request, area.width as usize);
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
            "1/y approve · 2/p remember · 3/n/esc deny".to_owned()
        } else if state.resume_picker.is_some() {
            "type search · enter resume · esc close · up/down select".to_owned()
        } else if !matching_slash_commands(state.input).is_empty() {
            "tab/up/down select · right complete · enter run".to_owned()
        } else if state.scroll_offset > 0 {
            "end to bottom · page up/down scroll".to_owned()
        } else if state.pending_model {
            match state.thinking_elapsed {
                Some(elapsed) => format!("{} · {}", state.status, format_elapsed(elapsed)),
                None => state.status.to_owned(),
            }
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

impl SlashComponent {
    fn render(&self, frame: &mut Frame, composer_area: Rect, state: &VisualState<'_>) {
        let matches = matching_slash_commands(state.input);
        if matches.is_empty() || state.pending_approval.is_some() || state.resume_picker.is_some() {
            return;
        }

        let max_width = frame.area().width.saturating_sub(2);
        let width = max_width.clamp(36, 74);
        let visible_count = matches.len().min(7);
        let height = (visible_count as u16).saturating_add(2);
        let x = composer_area.x;
        let y = composer_area.y.saturating_sub(height);
        let area = Rect::new(x, y, width, height);
        let selected = state.slash_selection.min(matches.len().saturating_sub(1));

        frame.render_widget(Clear, area);

        let mut lines = Vec::new();
        for (index, command) in visible_matches(&matches, selected, visible_count)
            .into_iter()
            .enumerate()
        {
            let absolute_index = slash_window_start(selected, visible_count) + index;
            let selected_row = absolute_index == selected;
            let marker = if selected_row { "› " } else { "  " };
            let style = if selected_row {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::Reset)
            };
            let usage_width = (area.width as usize).saturating_div(2).saturating_sub(4);
            let description_width = (area.width as usize)
                .saturating_sub(usage_width)
                .saturating_sub(7);
            lines.push(Line::from(vec![
                Span::styled(marker, style),
                Span::styled(truncate(command.usage, usage_width), style),
                Span::styled("  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    truncate(command.description, description_width),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }
}

fn visible_matches<'a>(
    matches: &[&'a SlashCommand],
    selected: usize,
    visible_count: usize,
) -> Vec<&'a SlashCommand> {
    let start = slash_window_start(selected, visible_count);
    let end = (start + visible_count).min(matches.len());
    matches[start..end].to_vec()
}

fn slash_window_start(selected: usize, visible_count: usize) -> usize {
    if selected >= visible_count {
        selected + 1 - visible_count
    } else {
        0
    }
}

impl ResumePickerComponent {
    fn render(&self, frame: &mut Frame, full: Rect, picker: &ResumePicker) {
        let area = full;
        frame.render_widget(Clear, area);

        let items = picker.filtered_items();
        let list_height = area.height.saturating_sub(5) as usize;
        let selected = picker.selected.min(items.len().saturating_sub(1));
        let start = if selected >= list_height && list_height > 0 {
            selected + 1 - list_height
        } else {
            0
        };
        let end = (start + list_height).min(items.len());
        let width = area.width as usize;
        let conversation_width = width.saturating_sub(41).max(12);

        let mut body: Vec<Line<'static>> = Vec::new();
        body.push(Line::from(vec![
            Span::styled(
                "Resume a previous session",
                Style::default().fg(Color::Reset),
            ),
            Span::raw("  "),
            Span::styled("Sort: Updated", Style::default().fg(Color::DarkGray)),
        ]));
        if picker.query.is_empty() {
            body.push(Line::from(Span::styled(
                "Type to search",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            body.push(Line::from(picker.query.clone()));
        }
        body.push(Line::from(vec![
            Span::styled("  Created      ", Style::default().fg(Color::DarkGray)),
            Span::styled("Updated      ", Style::default().fg(Color::DarkGray)),
            Span::styled("Branch  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Conversation", Style::default().fg(Color::DarkGray)),
        ]));

        if items.is_empty() {
            body.push(Line::from(Span::styled(
                "  No sessions found for this workspace.",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for (index, item) in items[start..end].iter().enumerate() {
                let absolute_index = start + index;
                let selected_row = absolute_index == selected;
                let marker = if selected_row { "› " } else { "  " };
                let style = if selected_row {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::Reset)
                };
                body.push(Line::from(vec![
                    Span::styled(marker, style),
                    Span::styled(pad_right(&item.created, 13), style),
                    Span::styled(pad_right(&item.updated_label, 13), style),
                    Span::styled(pad_right(&item.branch, 8), style),
                    Span::styled(truncate(&item.conversation, conversation_width), style),
                ]));
            }
        }

        frame.render_widget(Paragraph::new(body), area);
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
    pub call_id: agent_contracts::domain::CallId,
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
    let right = content_width
        .saturating_add(1)
        .saturating_sub(title.chars().count());

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

    if matches!(message.role, VisualRole::Assistant) {
        lines.extend(crate::markdown::render_assistant_markdown(
            &message.text,
            prefix,
            style,
            width,
        ));
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

fn append_approval_lines(
    lines: &mut Vec<Line<'static>>,
    request: &AppApprovalRequest,
    width: usize,
) {
    let safety = request
        .tool_spec
        .as_ref()
        .map(|spec| format!("{:?}", spec.safety))
        .unwrap_or_else(|| "unknown".to_owned());
    let args = compact_value(&request.call.args);
    let text_width = width.saturating_sub(4).max(1);

    lines.push(Line::from(vec![
        Span::styled("? ", Style::default().fg(Color::Yellow)),
        Span::styled(
            "Would you like to allow this tool call?",
            Style::default().fg(Color::Yellow),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  tool: "),
        Span::styled(
            request.call.name.clone(),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(format!(" · {safety}"), Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  cwd:  "),
        Span::styled(
            truncate(request.cwd.display().to_string(), text_width),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    for seg in wrap_text(&format!("reason: {}", request.reason), text_width) {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(seg, Style::default().fg(Color::DarkGray)),
        ]));
    }
    for seg in wrap_text(&format!("args: {args}"), text_width) {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(seg, Style::default().fg(Color::DarkGray)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("1. Yes, proceed", Style::default().fg(Color::Green)),
        Span::raw("  "),
        Span::styled(
            "2. Yes, remember exact call",
            Style::default().fg(Color::Green),
        ),
        Span::raw("  "),
        Span::styled("3. No", Style::default().fg(Color::Red)),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "y/н approve · p/з remember · n/т/esc deny",
            Style::default().fg(Color::DarkGray),
        ),
    ]));
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

fn pad_right(input: &str, width: usize) -> String {
    let truncated = truncate(input, width);
    let padding = width.saturating_sub(truncated.chars().count());
    format!("{truncated}{}", " ".repeat(padding))
}

fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_elapsed_for_footer_stopwatch() {
        assert_eq!(format_elapsed(Duration::from_secs(5)), "00:05");
        assert_eq!(format_elapsed(Duration::from_secs(125)), "02:05");
        assert_eq!(format_elapsed(Duration::from_secs(3_665)), "1:01:05");
    }

    #[test]
    fn session_card_borders_have_equal_width() {
        let state = VisualState {
            model: "anthropic/deepseek-v4-pro",
            cwd: Path::new("/tmp/workspace"),
            session_dir: None,
            messages: &[],
            input: "",
            footer: "",
            status: "ready",
            spinner_index: 0,
            scroll_offset: 0,
            pending_approval: None,
            pending_model: false,
            streaming: false,
            thinking_elapsed: None,
            resume_picker: None,
            slash_selection: 0,
        };

        let lines = session_card(&state, 80);
        let top_width = lines[0].width();
        let bottom_width = lines[4].width();

        assert_eq!(top_width, bottom_width);
    }
}
