use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use agent_contracts::app_protocol::AppApprovalRequest;
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    session_picker::ResumePicker,
    slash_commands::{SlashCommand, matching_slash_commands},
};

pub(crate) const STATUS_MARKER: &str = "•";

pub(crate) fn muted_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

pub(crate) struct VisualState<'a> {
    pub model: &'a str,
    pub cwd: &'a Path,
    pub session_label: &'a str,
    pub input: &'a str,
    pub input_paste_ranges: &'a [InputPasteRange],
    pub footer: &'a str,
    pub status: &'a str,
    pub pending_approval: Option<&'a AppApprovalRequest>,
    pub pending_model: bool,
    pub streaming: bool,
    pub streaming_message: Option<&'a VisualMessage>,
    pub reasoning_mode: ReasoningDisplayMode,
    pub reasoning_summary: &'a str,
    pub active_context_tokens: Option<u32>,
    pub active_output_tokens: Option<u32>,
    pub thinking_elapsed: Option<Duration>,
    pub resume_picker: Option<&'a ResumePicker>,
    pub context_report: Option<&'a str>,
    pub context_report_scroll: usize,
    pub slash_selection: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReasoningDisplayMode {
    Hidden,
    Summary,
    Expanded,
}

impl ReasoningDisplayMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Summary => "summary",
            Self::Expanded => "expanded",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputPasteRange {
    pub start: usize,
    pub end: usize,
    pub char_count: usize,
}

pub(crate) struct VisualSurface {
    resume_picker: ResumePickerComponent,
    context_report: ContextReportComponent,
}

impl Default for VisualSurface {
    fn default() -> Self {
        Self {
            resume_picker: ResumePickerComponent,
            context_report: ContextReportComponent,
        }
    }
}

impl VisualSurface {
    pub(crate) fn render_overlay(&self, frame: &mut Frame, state: &VisualState<'_>) {
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
    }
}

#[derive(Clone)]
struct DisplaySegment {
    text: String,
    style: Style,
}

struct ResumePickerComponent;
struct ContextReportComponent;

pub(crate) fn composer_lines(
    state: &VisualState<'_>,
    width: usize,
) -> (Vec<Line<'static>>, usize, usize) {
    let prompt = if state.pending_approval.is_some() {
        Span::styled("?", muted_style())
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
                Span::styled("Ask agent to do anything", muted_style()),
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

pub(crate) fn reasoning_preview_visible(state: &VisualState<'_>) -> bool {
    !matches!(state.reasoning_mode, ReasoningDisplayMode::Hidden)
        && !state.reasoning_summary.trim().is_empty()
}

pub(crate) fn append_reasoning_preview_lines(
    lines: &mut Vec<Line<'static>>,
    state: &VisualState<'_>,
    width: usize,
) {
    if !reasoning_preview_visible(state) {
        return;
    }

    let style = muted_style();
    lines.push(Line::from(Span::styled("◌ reasoning summary", style)));
    match state.reasoning_mode {
        ReasoningDisplayMode::Hidden => {}
        ReasoningDisplayMode::Summary => {
            let first_line = state
                .reasoning_summary
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or_default();
            append_plain_preview_text(lines, first_line, "  ", style, width);
            lines.push(Line::from(Span::styled(
                "  /reasoning opens full summary",
                style,
            )));
        }
        ReasoningDisplayMode::Expanded => {
            append_plain_preview_text(lines, state.reasoning_summary, "  ", style, width);
        }
    }
}

fn append_plain_preview_text(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    prefix: &str,
    style: Style,
    width: usize,
) {
    let mut first_segment = true;
    let text_width = width.saturating_sub(prefix.chars().count()).max(1);
    for source_line in text.lines() {
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

pub(crate) fn active_status_line(state: &VisualState<'_>, include_marker: bool) -> Line<'static> {
    let label = activity_label(state);
    let mut spans = Vec::new();
    if include_marker {
        spans.push(Span::styled(STATUS_MARKER.to_owned(), muted_style()));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(label, muted_style()));
    if let Some(elapsed) = state.thinking_elapsed {
        spans.push(Span::styled(" · ", muted_style()));
        spans.push(Span::styled(format_elapsed(elapsed), muted_style()));
    }
    if let Some(tokens) = state.active_output_tokens.filter(|tokens| *tokens > 0) {
        spans.push(Span::styled(" · ", muted_style()));
        spans.push(Span::styled(
            format!("↓ {}", format_token_count(tokens)),
            muted_style(),
        ));
    } else if let Some(tokens) = state.active_context_tokens.filter(|tokens| *tokens > 0) {
        spans.push(Span::styled(" · ", muted_style()));
        spans.push(Span::styled(
            format!("ctx {}", format_token_count(tokens)),
            muted_style(),
        ));
    }
    spans.push(Span::styled(" · esc cancel", muted_style()));
    Line::from(spans)
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

pub(crate) fn format_token_count(tokens: u32) -> String {
    if tokens >= 10_000 {
        format!("{:.1}k tokens", tokens as f64 / 1_000.0)
    } else if tokens >= 1_000 {
        let tenths = (tokens + 50) / 100;
        format!("{}.{}k tokens", tenths / 10, tenths % 10)
    } else {
        format!("{tokens} tokens")
    }
}

pub(crate) fn slash_plain_lines(state: &VisualState<'_>, width: usize) -> Vec<Line<'static>> {
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

    #[cfg(test)]
    pub(crate) fn draft(text: impl Into<String>) -> Self {
        Self {
            role: VisualRole::Draft,
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
    Draft,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolStatus {
    Running,
    Ok,
    Err,
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
        VisualRole::Draft => ("◦ draft ", muted_style()),
        VisualRole::System => ("  ", muted_style()),
        VisualRole::Error => ("! ", Style::default().fg(Color::Red)),
        VisualRole::Tool => ("  ", muted_style()),
    };

    let text_width = width.saturating_sub(prefix.chars().count()).max(1);
    if message.text.is_empty() {
        lines.push(Line::from(Span::styled(
            prefix.trim_end().to_owned(),
            style,
        )));
        return;
    }

    if matches!(message.role, VisualRole::Assistant | VisualRole::Draft) {
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

fn append_tool_card_lines(lines: &mut Vec<Line<'static>>, card: &ToolCard, width: usize) {
    let status = tool_status_style(card.status);
    let action = tool_action_body(card, status.label);
    lines.push(Line::from(vec![
        Span::styled(format!("{} ", status.marker), status.marker_style),
        Span::styled(status.label.to_owned(), status.label_style),
        Span::styled(" ", muted_style()),
        Span::styled(action, status.action_style),
    ]));

    if !card.output_preview.is_empty() {
        let preview_width = width.saturating_sub(4).max(1);
        let mut first_output_line = true;
        for raw in card.output_preview.lines() {
            for segment in wrap_text(raw, preview_width) {
                let prefix = if first_output_line { "  └ " } else { "    " };
                lines.push(Line::from(vec![
                    Span::styled(prefix, tool_output_prefix_style(card.status)),
                    Span::styled(segment, tool_output_style(card.status)),
                ]));
                first_output_line = false;
            }
        }
    }
}

pub(crate) struct ToolStatusStyle {
    pub marker: &'static str,
    pub marker_style: Style,
    pub label: &'static str,
    pub label_style: Style,
    pub action_style: Style,
}

pub(crate) fn tool_status_style(status: ToolStatus) -> ToolStatusStyle {
    match status {
        ToolStatus::Running => ToolStatusStyle {
            marker: "●",
            marker_style: Style::default().fg(Color::Yellow),
            label: "Running",
            label_style: Style::default().fg(Color::Yellow),
            action_style: Style::default().fg(Color::LightCyan),
        },
        ToolStatus::Ok => ToolStatusStyle {
            marker: "●",
            marker_style: Style::default().fg(Color::Green),
            label: "Ran",
            label_style: Style::default().fg(Color::Green),
            action_style: Style::default().fg(Color::LightCyan),
        },
        ToolStatus::Err => ToolStatusStyle {
            marker: "●",
            marker_style: Style::default().fg(Color::Red),
            label: "Error",
            label_style: Style::default().fg(Color::Red),
            action_style: Style::default().fg(Color::LightRed),
        },
    }
}

pub(crate) fn tool_action_body(card: &ToolCard, status_label: &str) -> String {
    let full = tool_action_text(card, status_label);
    full.strip_prefix(status_label)
        .map(str::trim_start)
        .filter(|body| !body.is_empty())
        .unwrap_or(&full)
        .to_owned()
}

pub(crate) fn tool_output_prefix_style(status: ToolStatus) -> Style {
    match status {
        ToolStatus::Err => Style::default().fg(Color::Red),
        ToolStatus::Running => Style::default().fg(Color::DarkGray),
        ToolStatus::Ok => Style::default().fg(Color::Cyan),
    }
}

pub(crate) fn tool_output_style(status: ToolStatus) -> Style {
    match status {
        ToolStatus::Err => Style::default().fg(Color::LightRed),
        ToolStatus::Running => muted_style(),
        ToolStatus::Ok => Style::default().fg(Color::Reset),
    }
}

pub(crate) fn tool_action_text(card: &ToolCard, status_label: &str) -> String {
    match card.status {
        ToolStatus::Running => {
            if !card.args_summary.is_empty() {
                format!("{status_label} {}", card.args_summary)
            } else {
                format!("{status_label} {}", card.name)
            }
        }
        ToolStatus::Ok => {
            if card.name == "shell" || card.name == "bash" {
                format!("{status_label} {}", card.args_summary)
            } else {
                card.args_summary.clone()
            }
        }
        ToolStatus::Err => {
            if card.args_summary.is_empty() {
                format!("{status_label} {}", card.name)
            } else {
                format!("{status_label} {}", card.args_summary)
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

pub(crate) fn tool_invocation_summary(name: &str, args: &serde_json::Value) -> String {
    match name {
        "shell" | "bash" => string_arg(args, "command")
            .or_else(|| string_arg(args, "cmd"))
            .map(|command| truncate(command, 140))
            .unwrap_or_else(|| compact_value(args)),
        "read_file" => path_action("Read", args),
        "write_file" => path_action("Wrote", args),
        "list_dir" => path_action("Listed", args),
        "grep" | "rg_search" => search_action(args),
        _ => {
            let args = compact_value(args);
            if args.is_empty() || args == "{}" {
                name.to_owned()
            } else {
                format!("{name} · {args}")
            }
        }
    }
}

fn path_action(action: &str, args: &serde_json::Value) -> String {
    string_arg(args, "path")
        .map(|path| format!("{action} {path}"))
        .unwrap_or_else(|| format!("{action} {}", compact_value(args)))
}

fn search_action(args: &serde_json::Value) -> String {
    let query = string_arg(args, "query")
        .or_else(|| string_arg(args, "pattern"))
        .unwrap_or_else(|| compact_value(args));
    if let Some(path) = string_arg(args, "path") {
        format!("Searched {path} for {query}")
    } else {
        format!("Searched {query}")
    }
}

fn string_arg(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)?.as_str().map(str::to_owned)
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

pub(crate) fn format_elapsed(elapsed: Duration) -> String {
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

pub(crate) fn display_path(path: &Path) -> String {
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
    use crate::{
        bottom_pane::{BottomPane, BottomPaneLines},
        cards::render_scrollback_header,
    };

    fn inline_panel_lines(state: &VisualState<'_>, width: usize) -> BottomPaneLines {
        BottomPane.lines(state, width)
    }

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
            pending_approval: None,
            pending_model: false,
            streaming: false,
            streaming_message: None,
            reasoning_mode: ReasoningDisplayMode::Hidden,
            reasoning_summary: "",
            active_context_tokens: None,
            active_output_tokens: None,
            thinking_elapsed: None,
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        let lines = render_scrollback_header(&state, 80);
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
    fn draft_message_is_labeled_and_muted_but_keeps_markdown() {
        let lines = render_scrollback_message(&VisualMessage::draft("Internal `plan`."), 80);

        assert_eq!(lines[0].spans[0].content.as_ref(), "◦ draft ");
        assert_eq!(lines[0].spans[0].style.fg, None);
        assert!(lines[0].spans[0].style.add_modifier.contains(Modifier::DIM));
        assert_eq!(lines[0].spans[2].content.as_ref(), "plan");
        assert_eq!(lines[0].spans[2].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn streaming_inline_panel_omits_active_answer_body() {
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
            pending_approval: None,
            pending_model: true,
            streaming: true,
            streaming_message: messages.last(),
            reasoning_mode: ReasoningDisplayMode::Hidden,
            reasoning_summary: "",
            active_context_tokens: None,
            active_output_tokens: Some(42),
            thinking_elapsed: None,
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        let panel = inline_panel_lines(&state, 80);
        let rendered = panel
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("responding"));
        assert!(rendered.contains("↓ 42 tokens"));
        assert!(!rendered.contains("line 30"));
        assert!(!rendered.contains("line 11"));
    }

    #[test]
    fn active_status_renders_above_input_while_streaming() {
        let messages = vec![VisualMessage::assistant("streaming answer")];
        let state = VisualState {
            model: "test/model",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "calling model...",
            pending_approval: None,
            pending_model: true,
            streaming: true,
            streaming_message: messages.first(),
            reasoning_mode: ReasoningDisplayMode::Hidden,
            reasoning_summary: "",
            active_context_tokens: None,
            active_output_tokens: Some(7),
            thinking_elapsed: Some(Duration::from_secs(12)),
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        let panel = inline_panel_lines(&state, 80);
        let rendered = panel
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let status_index = rendered
            .iter()
            .position(|line| line.contains("responding"))
            .expect("status line");

        assert_eq!(status_index, 0);
        assert!(rendered[status_index].contains("00:12"));
        assert!(rendered[status_index].contains("↓ 7 tokens"));
        assert_eq!(rendered[status_index + 1], "");
        assert!(rendered[status_index + 2].starts_with("› "));
        assert_eq!(rendered[status_index + 3], "");
        assert!(
            panel.lines[status_index]
                .spans
                .iter()
                .any(|span| span.content == "00:12"
                    && span.style.fg.is_none()
                    && span.style.add_modifier.contains(Modifier::DIM))
        );
    }

    #[test]
    fn reasoning_summary_mode_renders_compact_preview() {
        let state = VisualState {
            model: "test/model",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "reasoning...",
            pending_approval: None,
            pending_model: true,
            streaming: false,
            streaming_message: None,
            reasoning_mode: ReasoningDisplayMode::Summary,
            reasoning_summary: "Checked files.\nThen planned the edit.",
            active_context_tokens: None,
            active_output_tokens: None,
            thinking_elapsed: Some(Duration::from_secs(2)),
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        let rendered = inline_panel_lines(&state, 80)
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .any(|line| line.contains("reasoning summary"))
        );
        assert!(rendered.iter().any(|line| line.contains("Checked files.")));
        assert!(!rendered.iter().any(|line| line.contains("Then planned")));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("/reasoning opens full summary"))
        );
    }

    #[test]
    fn completed_shell_tool_renders_like_transcript_action() {
        let message = VisualMessage::tool(ToolCard {
            call_id: agent_contracts::domain::new_call_id(),
            name: "shell".to_owned(),
            args_summary: "uname -sr".to_owned(),
            status: ToolStatus::Ok,
            output_preview: "Linux 6.19.9\nextra".to_owned(),
        });

        let lines = render_scrollback_message(&message, 80)
            .into_iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(lines[0], "● Ran uname -sr");
        assert_eq!(lines[1], "  └ Linux 6.19.9");
        assert_eq!(lines[2], "    extra");

        let styled_lines = render_scrollback_message(&message, 80);
        assert_eq!(styled_lines[0].spans[0].style.fg, Some(Color::Green));
        assert_eq!(styled_lines[0].spans[1].style.fg, Some(Color::Green));
        assert_eq!(styled_lines[0].spans[3].style.fg, Some(Color::LightCyan));
        assert_eq!(styled_lines[1].spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(styled_lines[1].spans[1].style.fg, Some(Color::Reset));
    }

    #[test]
    fn failed_tool_uses_red_status_without_cross_marker() {
        let message = VisualMessage::tool(ToolCard {
            call_id: agent_contracts::domain::new_call_id(),
            name: "apply_patch".to_owned(),
            args_summary: r#"{"patch":"bad"}"#.to_owned(),
            status: ToolStatus::Err,
            output_preview: "plugin patch error".to_owned(),
        });

        let styled_lines = render_scrollback_message(&message, 80);
        let first_line = styled_lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(first_line, r#"● Error {"patch":"bad"}"#);
        assert!(!first_line.contains('✗'));
        assert_eq!(styled_lines[0].spans[0].style.fg, Some(Color::Red));
        assert_eq!(styled_lines[0].spans[1].style.fg, Some(Color::Red));
        assert_eq!(styled_lines[1].spans[0].style.fg, Some(Color::Red));
        assert_eq!(styled_lines[1].spans[1].style.fg, Some(Color::LightRed));
    }

    #[test]
    fn running_non_shell_tool_shows_arguments_when_available() {
        let message = VisualMessage::tool(ToolCard {
            call_id: agent_contracts::domain::new_call_id(),
            name: "list_dir".to_owned(),
            args_summary: r#"{"path":"."}"#.to_owned(),
            status: ToolStatus::Running,
            output_preview: String::new(),
        });

        let line = render_scrollback_message(&message, 80)[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(line, r#"● Running {"path":"."}"#);
    }

    #[test]
    fn tool_invocation_summary_uses_human_labels() {
        assert_eq!(
            tool_invocation_summary(
                "shell",
                &serde_json::json!({"command": "cargo check 2>&1 | head -100"})
            ),
            "cargo check 2>&1 | head -100"
        );
        assert_eq!(
            tool_invocation_summary("read_file", &serde_json::json!({"path": "Cargo.toml"})),
            "Read Cargo.toml"
        );
    }

    #[test]
    fn active_status_uses_stable_marker_instead_of_braille_animation() {
        let state = VisualState {
            model: "test/model",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "calling model...",
            pending_approval: None,
            pending_model: true,
            streaming: false,
            streaming_message: None,
            reasoning_mode: ReasoningDisplayMode::Hidden,
            reasoning_summary: "",
            active_context_tokens: None,
            active_output_tokens: None,
            thinking_elapsed: Some(Duration::from_secs(12)),
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        let line = active_status_line(&state, true);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.starts_with("• working"));
        assert!(
            !rendered
                .chars()
                .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch))
        );
    }

    #[test]
    fn idle_inline_panel_keeps_gap_above_input() {
        let state = VisualState {
            model: "test/model",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "ready",
            pending_approval: None,
            pending_model: false,
            streaming: false,
            streaming_message: None,
            reasoning_mode: ReasoningDisplayMode::Hidden,
            reasoning_summary: "",
            active_context_tokens: None,
            active_output_tokens: None,
            thinking_elapsed: None,
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        let panel = inline_panel_lines(&state, 80);
        let rendered = panel
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(rendered[0], "");
        assert!(rendered[1].starts_with("› "));
        assert_eq!(panel.cursor_row, 1);
    }

    #[test]
    fn typing_keeps_idle_composer_row_stable() {
        fn idle_state(input: &'static str) -> VisualState<'static> {
            VisualState {
                model: "test/model",
                cwd: Path::new("/tmp/workspace"),
                session_label: "1",
                input,
                input_paste_ranges: &[],
                footer: "enter send",
                status: "ready",
                pending_approval: None,
                pending_model: false,
                streaming: false,
                streaming_message: None,
                reasoning_mode: ReasoningDisplayMode::Hidden,
                reasoning_summary: "",
                active_context_tokens: None,
                active_output_tokens: None,
                thinking_elapsed: None,
                resume_picker: None,
                context_report: None,
                context_report_scroll: 0,
                slash_selection: 0,
            }
        }

        let empty = idle_state("");
        let typed = idle_state("a");

        let empty_panel = inline_panel_lines(&empty, 80);
        let typed_panel = inline_panel_lines(&typed, 80);

        assert_eq!(typed_panel.cursor_row, empty_panel.cursor_row);
        assert_eq!(typed_panel.lines.len(), empty_panel.lines.len());
    }

    #[test]
    fn streaming_inline_panel_does_not_render_transcript_messages() {
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
            pending_approval: None,
            pending_model: true,
            streaming: true,
            streaming_message: messages.first(),
            reasoning_mode: ReasoningDisplayMode::Hidden,
            reasoning_summary: "",
            active_context_tokens: None,
            active_output_tokens: None,
            thinking_elapsed: None,
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        let panel = inline_panel_lines(&state, 80);
        let rendered = panel
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("responding"));
        assert!(!rendered.contains("streaming answer"));
        assert!(!rendered.contains("later status"));
    }

    #[test]
    fn scrollback_message_renders_completed_markdown_and_keeps_tail_raw() {
        let messages = vec![VisualMessage::assistant(
            "Use `cargo test`.\nStill **streaming",
        )];
        let final_lines = render_scrollback_message(messages.first().unwrap(), 80);
        let rendered = final_lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|span| {
            span.content.as_ref() == "cargo test" && span.style.fg == Some(Color::Yellow)
        }));
        assert!(
            rendered
                .iter()
                .any(|span| span.content.as_ref().contains("Still"))
        );

        let rendered_text = final_lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered_text.iter().any(|line| line.contains("Use ")));
        assert!(rendered_text.iter().any(|line| line.contains("Still")));
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
            pending_approval: None,
            pending_model: false,
            streaming: false,
            streaming_message: None,
            reasoning_mode: ReasoningDisplayMode::Hidden,
            reasoning_summary: "",
            active_context_tokens: None,
            active_output_tokens: None,
            thinking_elapsed: None,
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            slash_selection: 0,
        };

        assert_eq!(
            crate::cards::footer_left_text(&state),
            "✓ done · 7s · enter send"
        );
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
