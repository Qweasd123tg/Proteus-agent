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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    session_picker::ResumePicker,
    slash_commands::{SlashCommand, matching_slash_commands},
};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(crate) struct VisualState<'a> {
    pub model: &'a str,
    pub cwd: &'a Path,
    pub session_label: &'a str,
    pub input: &'a str,
    pub input_paste_ranges: &'a [InputPasteRange],
    pub footer: &'a str,
    pub status: &'a str,
    pub spinner_index: usize,
    pub scroll_offset: usize,
    pub pending_approval: Option<&'a AppApprovalRequest>,
    pub pending_model: bool,
    pub streaming: bool,
    pub streaming_message: Option<&'a VisualMessage>,
    pub active_context_tokens: Option<u32>,
    pub active_output_tokens: Option<u32>,
    pub thinking_elapsed: Option<Duration>,
    pub resume_picker: Option<&'a ResumePicker>,
    pub context_report: Option<&'a str>,
    pub context_report_scroll: usize,
    pub slash_selection: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputPasteRange {
    pub start: usize,
    pub end: usize,
    pub char_count: usize,
}

pub(crate) struct VisualSurface {
    composer: ComposerComponent,
    footer: FooterComponent,
    resume_picker: ResumePickerComponent,
    context_report: ContextReportComponent,
    slash: SlashComponent,
}

impl Default for VisualSurface {
    fn default() -> Self {
        Self {
            composer: ComposerComponent,
            footer: FooterComponent,
            resume_picker: ResumePickerComponent,
            context_report: ContextReportComponent,
            slash: SlashComponent,
        }
    }
}

impl VisualSurface {
    pub(crate) fn render_inline(&self, frame: &mut Frame, state: &VisualState<'_>) {
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
        if let Some(report) = state.context_report {
            self.context_report
                .render(frame, frame.area(), report, state.context_report_scroll);
            return;
        }

        let approval_height = if state.pending_approval.is_some() {
            9u16.min(frame.area().height.saturating_sub(3))
        } else {
            0
        };
        let live_height = live_preview_height(
            state,
            frame.area().height.saturating_sub(approval_height + 3),
        );
        let total_height = approval_height
            .saturating_add(live_height)
            .saturating_add(3)
            .min(frame.area().height);
        let bottom = Rect::new(
            frame.area().x,
            frame.area().bottom().saturating_sub(total_height),
            frame.area().width,
            total_height,
        );
        frame.render_widget(Clear, bottom);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(approval_height),
                Constraint::Length(live_height),
                Constraint::Length(2),
                Constraint::Length(1),
            ])
            .split(bottom);

        if let Some(request) = state.pending_approval {
            let mut approval_lines = Vec::new();
            append_approval_lines(&mut approval_lines, request, chunks[0].width as usize);
            frame.render_widget(Paragraph::new(approval_lines), chunks[0]);
        }

        if live_height > 0 {
            let live_lines =
                live_status_lines(state, chunks[1].width as usize, live_height as usize);
            frame.render_widget(Paragraph::new(live_lines), chunks[1]);
        }

        self.composer.render(frame, chunks[2], state);
        self.footer.render(frame, chunks[3], state);
        self.slash.render(frame, chunks[2], state);

        let cursor_x = chunks[2].x + 2 + state.input.chars().count() as u16;
        let cursor_y = chunks[2].y;
        frame.set_cursor_position(Position::new(
            cursor_x.min(chunks[2].right().saturating_sub(1)),
            cursor_y,
        ));
    }
}

pub(crate) struct InlinePanelLines {
    pub lines: Vec<Line<'static>>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

#[derive(Clone)]
struct DisplaySegment {
    text: String,
    style: Style,
}

pub(crate) fn inline_panel_lines(
    state: &VisualState<'_>,
    width: usize,
    max_live_lines: usize,
) -> InlinePanelLines {
    let mut lines = Vec::new();

    if !matching_slash_commands(state.input).is_empty()
        && state.pending_approval.is_none()
        && state.resume_picker.is_none()
    {
        lines.extend(slash_plain_lines(state, width));
        lines.push(Line::raw(""));
    }

    if let Some(request) = state.pending_approval {
        let mut approval_lines = Vec::new();
        append_approval_lines(&mut approval_lines, request, width);
        lines.extend(approval_lines);
    } else if state.streaming || state.pending_model {
        lines.extend(live_status_lines(state, width, max_live_lines));
    }

    let composer_start = lines.len();
    let (composer_lines, composer_cursor_row, cursor_col) = composer_lines(state, width);
    lines.extend(composer_lines);
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        footer_plain_line(state, width),
        Style::default().fg(Color::DarkGray),
    )));

    InlinePanelLines {
        lines,
        cursor_row: composer_start + composer_cursor_row,
        cursor_col,
    }
}

trait VisualComponent {
    fn render(&self, frame: &mut Frame, area: Rect, state: &VisualState<'_>);
}

struct ComposerComponent;
struct FooterComponent;
struct ResumePickerComponent;
struct ContextReportComponent;
struct SlashComponent;

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

fn composer_lines(state: &VisualState<'_>, width: usize) -> (Vec<Line<'static>>, usize, usize) {
    let prompt = if state.pending_approval.is_some() {
        Span::styled("?", Style::default().fg(Color::Yellow))
    } else {
        Span::styled("›", Style::default().fg(Color::Cyan))
    };
    let available_width = width.saturating_sub(1).max(1);
    let prompt_width = 2usize.min(available_width);

    if state.input.is_empty() && !state.pending_model {
        return (
            vec![Line::from(vec![
                prompt,
                Span::raw(" "),
                Span::styled(
                    "Ask agent to do anything",
                    Style::default().fg(Color::DarkGray),
                ),
            ])],
            0,
            prompt_width,
        );
    }

    let segments =
        display_segments_from_paste_ranges(state.input, state.input_paste_ranges, Style::default());
    let wrapped = wrap_segments_for_width(&segments, available_width, prompt_width);
    let mut lines = Vec::new();
    for (idx, segments) in wrapped.lines.iter().enumerate() {
        if idx == 0 {
            let mut spans = vec![prompt.clone(), Span::raw(" ")];
            spans.extend(segments.clone());
            lines.push(Line::from(spans));
        } else {
            let mut spans = vec![Span::raw("  ")];
            spans.extend(segments.clone());
            lines.push(Line::from(spans));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(vec![prompt, Span::raw(" ")]));
    }
    (lines, wrapped.cursor_row, wrapped.cursor_col)
}

struct WrappedInput {
    lines: Vec<Vec<Span<'static>>>,
    cursor_row: usize,
    cursor_col: usize,
}

fn wrap_segments_for_width(
    segments: &[DisplaySegment],
    width: usize,
    first_prefix_width: usize,
) -> WrappedInput {
    let first_limit = width.saturating_sub(first_prefix_width).max(1);
    let next_prefix_width = 2usize.min(width);
    let next_limit = width.saturating_sub(next_prefix_width).max(1);
    let mut lines = Vec::new();
    let mut current = Vec::<Span<'static>>::new();
    let mut used = 0usize;
    let mut first = true;

    for segment in segments {
        for ch in segment.text.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                lines.push(current);
                current = Vec::new();
                used = 0;
                first = false;
                continue;
            }

            let ch_width = ch.width().unwrap_or(0);
            let limit = if first { first_limit } else { next_limit };
            if used > 0 && used + ch_width > limit {
                lines.push(current);
                current = Vec::new();
                used = 0;
                first = false;
            }
            push_styled_char(&mut current, ch, segment.style);
            used += ch_width;
        }
    }
    lines.push(current);

    let cursor_row = lines.len().saturating_sub(1);
    let prefix_width = if cursor_row == 0 {
        first_prefix_width
    } else {
        next_prefix_width
    };
    let cursor_col = prefix_width + line_width(&lines[cursor_row]);

    WrappedInput {
        lines,
        cursor_row,
        cursor_col,
    }
}

fn display_segments_from_paste_ranges(
    text: &str,
    ranges: &[InputPasteRange],
    normal_style: Style,
) -> Vec<DisplaySegment> {
    let mut segments = Vec::new();
    let mut cursor = 0usize;
    for range in ranges {
        if range.start < cursor || range.end > text.len() || range.start > range.end {
            continue;
        }
        if cursor < range.start {
            segments.push(DisplaySegment {
                text: text[cursor..range.start].to_owned(),
                style: normal_style,
            });
        }
        segments.push(DisplaySegment {
            text: format!("[Pasted Content {} chars]", range.char_count),
            style: paste_marker_style(),
        });
        cursor = range.end;
    }
    if cursor < text.len() {
        segments.push(DisplaySegment {
            text: text[cursor..].to_owned(),
            style: normal_style,
        });
    }
    if segments.is_empty() {
        segments.push(DisplaySegment {
            text: text.to_owned(),
            style: normal_style,
        });
    }
    segments
}

fn push_styled_char(spans: &mut Vec<Span<'static>>, ch: char, style: Style) {
    if let Some(last) = spans.last_mut()
        && last.style == style
    {
        last.content.to_mut().push(ch);
        return;
    }
    spans.push(Span::styled(ch.to_string(), style));
}

fn line_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn paste_marker_style() -> Style {
    Style::default().fg(Color::Blue)
}

impl VisualComponent for FooterComponent {
    fn render(&self, frame: &mut Frame, area: Rect, state: &VisualState<'_>) {
        let left = footer_left_text(state);
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

fn footer_plain_line(state: &VisualState<'_>, width: usize) -> String {
    let left = footer_left_text(state);
    truncate(format!("  {left}    {}", state.footer), width)
}

fn footer_left_text(state: &VisualState<'_>) -> String {
    if state.pending_approval.is_some() {
        "1/y approve · 2/p remember · 3/n/esc deny".to_owned()
    } else if state.resume_picker.is_some() {
        "type search · enter resume · esc close · up/down select".to_owned()
    } else if !matching_slash_commands(state.input).is_empty() {
        "enter/tab complete · up/down select · enter run exact".to_owned()
    } else if state.scroll_offset > 0 {
        "end to bottom · page up/down scroll".to_owned()
    } else if state.pending_model {
        active_turn_line(state, false)
    } else if let Some(done) = state.status.strip_prefix("done") {
        format!("✓ done{} · enter send", done)
    } else {
        state.status.to_owned()
    }
}

fn live_preview_height(state: &VisualState<'_>, available: u16) -> u16 {
    if available == 0 || state.pending_approval.is_some() {
        return 0;
    }
    if state.streaming {
        return available.max(1);
    }
    if state.pending_model {
        return 1.min(available);
    }
    0
}

fn live_status_lines(
    state: &VisualState<'_>,
    width: usize,
    max_lines: usize,
) -> Vec<Line<'static>> {
    if let Some(message) = state.streaming_message {
        let mut body = Vec::new();
        append_message_lines(&mut body, message, width);
        let mut lines = vec![Line::from(vec![
            Span::styled("+ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                active_turn_line(state, false),
                Style::default().fg(Color::Yellow),
            ),
        ])];
        if max_lines > 1 {
            lines.extend(tail_lines(body, max_lines.saturating_sub(1).max(1)));
        }
        return lines;
    }

    if state.pending_model {
        return vec![Line::from(vec![
            Span::styled("+ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                active_turn_line(state, true),
                Style::default().fg(Color::Yellow),
            ),
        ])];
    }

    Vec::new()
}

fn active_turn_line(state: &VisualState<'_>, include_spinner: bool) -> String {
    let label = activity_label(state);
    let mut parts = Vec::new();
    if include_spinner {
        parts.push(SPINNER[state.spinner_index % SPINNER.len()].to_owned());
    }
    parts.push(label);
    if let Some(elapsed) = state.thinking_elapsed {
        parts.push(format_elapsed(elapsed));
    }
    if let Some(tokens) = state.active_output_tokens.filter(|tokens| *tokens > 0) {
        parts.push(format!("↓ {}", format_token_count(tokens)));
    } else if let Some(tokens) = state.active_context_tokens.filter(|tokens| *tokens > 0) {
        parts.push(format!("ctx {}", format_token_count(tokens)));
    }
    parts.push("esc cancel".to_owned());
    parts.join(" · ")
}

fn activity_label(state: &VisualState<'_>) -> String {
    let status = state.status.trim();
    if state.streaming {
        "responding".to_owned()
    } else if status == "sent" {
        "sent".to_owned()
    } else if status == "request accepted" || status.starts_with("context") {
        "preparing".to_owned()
    } else if status.starts_with("tool:") {
        status.to_owned()
    } else if status == "cancel requested" {
        "canceling".to_owned()
    } else if status == "finishing" {
        "finishing".to_owned()
    } else {
        "working".to_owned()
    }
}

fn format_token_count(tokens: u32) -> String {
    if tokens >= 10_000 {
        format!("{:.1}k tokens", tokens as f64 / 1_000.0)
    } else if tokens >= 1_000 {
        let tenths = (tokens + 50) / 100;
        format!("{}.{}k tokens", tenths / 10, tenths % 10)
    } else {
        format!("{tokens} tokens")
    }
}

fn slash_plain_lines(state: &VisualState<'_>, width: usize) -> Vec<Line<'static>> {
    let matches = matching_slash_commands(state.input);
    let visible_count = matches.len().min(7);
    let selected = state.slash_selection.min(matches.len().saturating_sub(1));
    let panel_width = width.clamp(36, 74);
    let mut out = Vec::new();
    out.push(Line::from(Span::styled(
        format!("┌{}┐", "─".repeat(panel_width.saturating_sub(2))),
        Style::default().fg(Color::DarkGray),
    )));
    for (index, command) in visible_matches(&matches, selected, visible_count)
        .into_iter()
        .enumerate()
    {
        let absolute_index = slash_window_start(selected, visible_count) + index;
        let selected_row = absolute_index == selected;
        let marker = if absolute_index == selected {
            "› "
        } else {
            "  "
        };
        let primary = if selected_row {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::Reset)
        };
        let muted = if selected_row {
            Style::default().fg(Color::Blue)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let usage_width = panel_width.saturating_div(2).saturating_sub(4);
        let description_width = panel_width.saturating_sub(usage_width).saturating_sub(7);
        let usage = truncate(command.usage, usage_width);
        let description = truncate(command.description, description_width);
        let content_width = panel_width.saturating_sub(2);
        let used = marker.chars().count() + usage.chars().count() + 2 + description.chars().count();
        out.push(Line::from(vec![
            Span::styled("│", Style::default().fg(Color::DarkGray)),
            Span::styled(marker.to_owned(), primary),
            Span::styled(usage, primary),
            Span::styled("  ", Style::default().fg(Color::DarkGray)),
            Span::styled(description, muted),
            Span::raw(" ".repeat(content_width.saturating_sub(used))),
            Span::styled("│", Style::default().fg(Color::DarkGray)),
        ]));
    }
    out.push(Line::from(Span::styled(
        format!("└{}┘", "─".repeat(panel_width.saturating_sub(2))),
        Style::default().fg(Color::DarkGray),
    )));
    out
}

fn tail_lines(mut lines: Vec<Line<'static>>, limit: usize) -> Vec<Line<'static>> {
    if lines.len() <= limit {
        return lines;
    }
    lines.drain(0..lines.len() - limit);
    lines
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

impl ContextReportComponent {
    fn render(&self, frame: &mut Frame, full: Rect, report: &str, scroll: usize) {
        frame.render_widget(Clear, full);
        let width = full.width as usize;
        let mut body = Vec::<Line<'static>>::new();
        body.push(Line::from(vec![
            Span::styled("Context Usage", Style::default().fg(Color::Reset)),
            Span::raw("  "),
            Span::styled("Esc close", Style::default().fg(Color::DarkGray)),
        ]));
        body.push(Line::raw(""));

        let content_width = width.saturating_sub(1).max(1);
        let content_height = full.height.saturating_sub(2) as usize;
        let rendered =
            crate::markdown::render_assistant_markdown(report, "", Style::default(), content_width);
        let max_scroll = rendered.len().saturating_sub(content_height);
        let start = scroll.min(max_scroll);
        body.extend(rendered.into_iter().skip(start).take(content_height));

        frame.render_widget(Paragraph::new(body), full);
    }
}

#[derive(Clone)]
pub(crate) struct VisualMessage {
    pub role: VisualRole,
    pub text: String,
    pub paste_ranges: Vec<InputPasteRange>,
    pub tool: Option<ToolCard>,
}

impl VisualMessage {
    pub(crate) fn user(text: impl Into<String>) -> Self {
        Self::user_with_paste_ranges(text, Vec::new())
    }

    pub(crate) fn user_with_paste_ranges(
        text: impl Into<String>,
        paste_ranges: Vec<InputPasteRange>,
    ) -> Self {
        Self {
            role: VisualRole::User,
            text: text.into(),
            paste_ranges,
            tool: None,
        }
    }

    pub(crate) fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: VisualRole::Assistant,
            text: text.into(),
            paste_ranges: Vec::new(),
            tool: None,
        }
    }

    pub(crate) fn system(text: impl Into<String>) -> Self {
        Self {
            role: VisualRole::System,
            text: text.into(),
            paste_ranges: Vec::new(),
            tool: None,
        }
    }

    pub(crate) fn error(text: impl Into<String>) -> Self {
        Self {
            role: VisualRole::Error,
            text: text.into(),
            paste_ranges: Vec::new(),
            tool: None,
        }
    }

    pub(crate) fn tool(card: ToolCard) -> Self {
        Self {
            role: VisualRole::Tool,
            text: String::new(),
            paste_ranges: Vec::new(),
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
    let rows = [
        format!("model:     {}", state.model),
        format!("directory: {cwd}"),
        format!("session:   {}", state.session_label),
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

    if matches!(message.role, VisualRole::User) && !message.paste_ranges.is_empty() {
        let segments =
            display_segments_from_paste_ranges(&message.text, &message.paste_ranges, style);
        let wrapped = wrap_segments_for_width(&segments, text_width, 0);
        let mut first_segment = true;
        for segments in wrapped.lines {
            let line_prefix = if first_segment { prefix } else { "  " };
            let mut spans = vec![Span::styled(line_prefix.to_owned(), style)];
            spans.extend(segments);
            lines.push(Line::from(spans));
            first_segment = false;
        }
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

pub(crate) fn render_scrollback_message(
    message: &VisualMessage,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    append_message_lines(&mut lines, message, width);
    lines.push(Line::raw(""));
    lines
}

pub(crate) fn render_scrollback_header(
    state: &VisualState<'_>,
    width: usize,
) -> Vec<Line<'static>> {
    session_card(state, width)
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
            session_label: "not persisted",
            input: "",
            input_paste_ranges: &[],
            footer: "",
            status: "ready",
            spinner_index: 0,
            scroll_offset: 0,
            pending_approval: None,
            pending_model: false,
            streaming: false,
            streaming_message: None,
            active_context_tokens: None,
            active_output_tokens: None,
            thinking_elapsed: None,
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        let lines = session_card(&state, 80);
        let top_width = lines[0].width();
        let bottom_width = lines[4].width();

        assert_eq!(top_width, bottom_width);
    }

    #[test]
    fn scrollback_message_keeps_markdown_spans() {
        let lines = render_scrollback_message(&VisualMessage::assistant("Use `cargo test`."), 80);

        assert_eq!(lines[0].spans[0].content.as_ref(), "• ");
        assert_eq!(lines[0].spans[2].content.as_ref(), "cargo test");
        assert_eq!(lines[0].spans[2].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn streaming_inline_preview_uses_available_height() {
        let text = (1..=30)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = vec![VisualMessage::assistant(text)];
        let state = VisualState {
            model: "test/model",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "",
            status: "calling model...",
            spinner_index: 0,
            scroll_offset: 0,
            pending_approval: None,
            pending_model: true,
            streaming: true,
            streaming_message: messages.last(),
            active_context_tokens: None,
            active_output_tokens: Some(42),
            thinking_elapsed: None,
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        let panel = inline_panel_lines(&state, 80, 20);
        let rendered = panel
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("line 30"));
        assert!(rendered.contains("responding"));
        assert!(rendered.contains("↓ 42 tokens"));
        assert!(rendered.contains("line 12"));
        assert!(!rendered.contains("line 11"));
    }

    #[test]
    fn streaming_inline_preview_uses_streaming_message_not_last_message() {
        let messages = vec![
            VisualMessage::assistant("streaming answer"),
            VisualMessage::system("later status"),
        ];
        let state = VisualState {
            model: "test/model",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "",
            status: "calling model...",
            spinner_index: 0,
            scroll_offset: 0,
            pending_approval: None,
            pending_model: true,
            streaming: true,
            streaming_message: messages.first(),
            active_context_tokens: None,
            active_output_tokens: None,
            thinking_elapsed: None,
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        let panel = inline_panel_lines(&state, 80, 20);
        let rendered = panel
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("streaming answer"));
        assert!(rendered.contains("responding"));
        assert!(!rendered.contains("later status"));
    }

    #[test]
    fn idle_footer_makes_recent_completion_explicit() {
        let state = VisualState {
            model: "test/model",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "done · 7s",
            spinner_index: 0,
            scroll_offset: 0,
            pending_approval: None,
            pending_model: false,
            streaming: false,
            streaming_message: None,
            active_context_tokens: None,
            active_output_tokens: None,
            thinking_elapsed: None,
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        assert_eq!(footer_left_text(&state), "✓ done · 7s · enter send");
    }

    #[test]
    fn user_paste_marker_keeps_surrounding_text_and_style() {
        let text = "before very large pasted text after";
        let lines = render_scrollback_message(
            &VisualMessage::user_with_paste_ranges(
                text,
                vec![InputPasteRange {
                    start: 7,
                    end: 29,
                    char_count: 28164,
                }],
            ),
            120,
        );

        let line = &lines[0];
        assert!(line.spans.iter().any(|span| span.content == "before "));
        assert!(line.spans.iter().any(|span| span.content == " after"));
        let marker = line
            .spans
            .iter()
            .find(|span| span.content == "[Pasted Content 28164 chars]")
            .expect("marker");
        assert_eq!(marker.style.fg, Some(Color::Blue));
    }
}
