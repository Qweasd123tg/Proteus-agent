use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use agent_contracts::app_protocol::AppApprovalRequest;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    config_summary::ConfigSummary,
    plan_intake::PlanIntakeState,
    session_picker::ResumePicker,
    slash_commands::{SlashCommand, matching_slash_commands},
};

mod overlay;
mod scrollback;

pub(crate) use overlay::VisualSurface;
pub(crate) use scrollback::{
    ToolCard, ToolStatus, VisualMessage, VisualRole, compact_value, render_scrollback_message,
    render_tool_card_lines, tool_action_body, tool_invocation_summary, tool_output_prefix_style,
    tool_output_style, tool_status_style, truncate, wrap_text,
};

pub(crate) const STATUS_MARKER: &str = "•";

pub(crate) fn muted_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

pub(crate) fn plan_review_lines(state: &VisualState<'_>, width: usize) -> Vec<Line<'static>> {
    let Some(review) = state.plan_review else {
        return Vec::new();
    };
    let selected = review
        .selected
        .min(PLAN_REVIEW_ACTIONS.len().saturating_sub(1));
    let mut lines = Vec::with_capacity(PLAN_REVIEW_ACTIONS.len());
    let available_width = width.saturating_sub(4).max(1);
    for (index, action) in PLAN_REVIEW_ACTIONS.iter().copied().enumerate() {
        let marker = if index == selected { "> " } else { "  " };
        let style = if index == selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(marker, style),
            Span::styled(truncate(action.label(), available_width), style),
        ]));
    }
    lines
}

pub(crate) fn plan_intake_lines(state: &VisualState<'_>, width: usize) -> Vec<Line<'static>> {
    let Some(intake) = state.plan_intake else {
        return Vec::new();
    };
    let question = intake.current_question();
    let mut lines = Vec::new();
    let mut progress = vec![Span::styled("←  ", muted_style())];
    for (index, _question) in intake.request().questions.iter().enumerate() {
        let marker = if index == intake.question_index() {
            "●"
        } else if index < intake.question_index() {
            "✔"
        } else {
            "☐"
        };
        let style = if index == intake.question_index() {
            Style::default().fg(Color::Cyan)
        } else {
            muted_style()
        };
        progress.push(Span::styled(
            truncate(
                &format!("{marker} {}", intake.question_header(index)),
                width.saturating_sub(8).max(1),
            ),
            style,
        ));
        progress.push(Span::raw("  "));
    }
    let submit_marker = if intake.is_last_question() {
        "✔"
    } else {
        "☐"
    };
    progress.push(Span::styled(
        format!("{submit_marker} Submit  →"),
        muted_style(),
    ));
    lines.push(Line::from(progress));
    lines.push(Line::from(vec![Span::styled(
        truncate(
            &format!(
                "Planning choices  {}/{}  {}",
                intake.question_index() + 1,
                intake.question_count(),
                intake.request().title
            ),
            width.max(1),
        ),
        muted_style(),
    )]));
    lines.push(Line::from(vec![Span::styled(
        truncate(&question.prompt, width.max(1)),
        Style::default().add_modifier(Modifier::BOLD),
    )]));

    let available_width = width.saturating_sub(4).max(1);
    for (index, option) in question.options.iter().enumerate() {
        let marker = if index == intake.current_selection() {
            "› "
        } else {
            "  "
        };
        let style = if index == intake.current_selection() {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let select_prefix = if question.multi_select {
            if intake.current_option_is_selected(index) {
                "[x] "
            } else {
                "[ ] "
            }
        } else {
            ""
        };
        let label = option
            .description
            .as_ref()
            .filter(|description| !description.is_empty())
            .map(|description| {
                format!(
                    "{}{}. {} - {}",
                    select_prefix,
                    index + 1,
                    option.label,
                    description
                )
            })
            .unwrap_or_else(|| format!("{}{}. {}", select_prefix, index + 1, option.label));
        lines.push(Line::from(vec![
            Span::styled(marker, style),
            Span::styled(truncate(&label, available_width), style),
        ]));
    }
    if question.allow_custom {
        let custom_index = intake.custom_index(intake.question_index());
        let marker = if intake.current_selection() == custom_index {
            "› "
        } else {
            "  "
        };
        let style = if intake.current_selection() == custom_index {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let custom = intake.current_custom_answer();
        let label = if custom.is_empty() {
            format!("{}. Type something.", custom_index + 1)
        } else {
            format!("{}. Type something: {custom}", custom_index + 1)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, style),
            Span::styled(truncate(&label, available_width), style),
        ]));
    }
    lines.push(Line::from(vec![Span::styled(
        "────────────────────────────────────────────────────────────────",
        muted_style(),
    )]));
    for (index, label) in [
        (
            intake.chat_index(intake.question_index()),
            "Chat about this",
        ),
        (
            intake.skip_index(intake.question_index()),
            "Skip interview and plan immediately",
        ),
    ] {
        let marker = if intake.current_selection() == index {
            "› "
        } else {
            "  "
        };
        let style = if intake.current_selection() == index {
            Style::default().fg(Color::Cyan)
        } else {
            muted_style()
        };
        lines.push(Line::from(vec![
            Span::styled(marker, style),
            Span::styled(
                truncate(&format!("{}. {label}", index + 1), available_width),
                style,
            ),
        ]));
    }
    lines.push(Line::from(vec![Span::styled(
        if question.multi_select {
            "Up/Down move · Space toggle · Left/Right question · Enter next/submit · type custom · Esc cancel"
        } else {
            "Up/Down choose · Left/Right question · Enter next/submit · type for custom · Esc cancel"
        },
        muted_style(),
    )]));
    lines
}

pub(crate) fn active_tool_lines(state: &VisualState<'_>, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (index, card) in state.active_tools.iter().enumerate() {
        if index > 0 {
            lines.push(Line::raw(""));
        }
        lines.extend(render_tool_card_lines(card, width));
    }
    lines
}

pub(crate) struct VisualState<'a> {
    pub model: &'a str,
    pub permission_mode: &'a str,
    pub cwd: &'a Path,
    pub session_label: &'a str,
    pub input: &'a str,
    pub input_paste_ranges: &'a [InputPasteRange],
    pub footer: &'a str,
    pub status: &'a str,
    pub pending_approval: Option<&'a AppApprovalRequest>,
    pub pending_model: bool,
    pub active_tools: &'a [ToolCard],
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
    pub config_summary: Option<&'a ConfigSummary>,
    pub config_summary_scroll: usize,
    pub plan_intake: Option<&'a PlanIntakeState>,
    pub plan_review: Option<PlanReviewVisualState>,
    pub slash_selection: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlanReviewVisualState {
    pub selected: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanReviewAction {
    ExecuteAuto,
    ExecuteNormal,
    Revise,
    Dismiss,
}

impl PlanReviewAction {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ExecuteAuto => "Submit + execute",
            Self::ExecuteNormal => "Execute with approvals",
            Self::Revise => "Revise",
            Self::Dismiss => "Dismiss",
        }
    }
}

pub(crate) const PLAN_REVIEW_ACTIONS: &[PlanReviewAction] = &[
    PlanReviewAction::ExecuteAuto,
    PlanReviewAction::ExecuteNormal,
    PlanReviewAction::Revise,
    PlanReviewAction::Dismiss,
];

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

#[derive(Clone)]
struct DisplaySegment {
    text: String,
    style: Style,
}

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
        plan_intake::PlanIntakeState,
    };
    use serde_json::json;

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
            permission_mode: "normal",
            cwd: Path::new("/tmp/workspace"),
            session_label: "not persisted",
            input: "",
            input_paste_ranges: &[],
            footer: "",
            status: "ready",
            pending_approval: None,
            pending_model: false,
            active_tools: &[],
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
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: None,
            plan_review: None,
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
            permission_mode: "normal",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "",
            status: "calling model...",
            pending_approval: None,
            pending_model: true,
            active_tools: &[],
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
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: None,
            plan_review: None,
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
            permission_mode: "normal",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "calling model...",
            pending_approval: None,
            pending_model: true,
            active_tools: &[],
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
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: None,
            plan_review: None,
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
            permission_mode: "normal",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "reasoning...",
            pending_approval: None,
            pending_model: true,
            active_tools: &[],
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
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: None,
            plan_review: None,
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

        assert_eq!(lines[0], "● ran uname -sr");
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

        assert_eq!(first_line, r#"● failed {"patch":"bad"}"#);
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

        assert_eq!(line, r#"● running {"path":"."}"#);
    }

    #[test]
    fn active_tool_card_renders_above_composer_before_finish() {
        let active_tools = vec![ToolCard {
            call_id: agent_contracts::domain::new_call_id(),
            name: "read_file".to_owned(),
            args_summary: "Read Cargo.toml".to_owned(),
            status: ToolStatus::Running,
            output_preview: String::new(),
        }];
        let state = VisualState {
            model: "test/model",
            permission_mode: "normal",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "tool: read_file",
            pending_approval: None,
            pending_model: true,
            active_tools: &active_tools,
            streaming: false,
            streaming_message: None,
            reasoning_mode: ReasoningDisplayMode::Hidden,
            reasoning_summary: "",
            active_context_tokens: None,
            active_output_tokens: None,
            thinking_elapsed: Some(Duration::from_secs(3)),
            resume_picker: None,
            context_report: None,
            context_report_scroll: 0,
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: None,
            plan_review: None,
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

        assert_eq!(rendered[0], "● running Read Cargo.toml");
        assert!(rendered.iter().any(|line| line.contains("tool: read_file")));
        assert!(matches!(
            panel.lines[0].spans[0].style.fg,
            Some(Color::Rgb(255, 149, 0)) | None
        ));
        assert_eq!(
            panel.lines[0].spans[1].style.fg,
            Some(Color::Rgb(255, 149, 0))
        );
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
            permission_mode: "normal",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "calling model...",
            pending_approval: None,
            pending_model: true,
            active_tools: &[],
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
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: None,
            plan_review: None,
            slash_selection: 0,
        };

        let panel = inline_panel_lines(&state, 80);
        let line = panel.lines.first().expect("active status line");
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
            permission_mode: "normal",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "ready",
            pending_approval: None,
            pending_model: false,
            active_tools: &[],
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
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: None,
            plan_review: None,
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
    fn plan_review_renders_selectable_actions_above_input() {
        let state = VisualState {
            model: "test/model",
            permission_mode: "plan",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "plan ready",
            pending_approval: None,
            pending_model: false,
            active_tools: &[],
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
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: None,
            plan_review: Some(PlanReviewVisualState { selected: 1 }),
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
                .any(|line| line.starts_with("> Execute with approvals"))
        );
        assert!(
            rendered
                .iter()
                .all(|line| !line.contains("read/write tools"))
        );
        assert!(rendered.iter().any(|line| line.starts_with("› ")));
    }

    #[test]
    fn plan_intake_renders_current_question_and_options() {
        let intake = PlanIntakeState::from_metadata(&json!({
            "plan_intake": {
                "id": "telegram-bot",
                "title": "Telegram bot",
                "questions": [{
                    "id": "stack",
                    "header": "Stack",
                    "prompt": "Какой stack?",
                    "options": [
                        {"id": "api", "label": "Telegram Bot API"},
                        {"id": "aiogram", "label": "aiogram"}
                    ],
                    "allow_custom": true
                }]
            }
        }))
        .expect("intake");
        let state = VisualState {
            model: "test/model",
            permission_mode: "plan",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "planning choices",
            pending_approval: None,
            pending_model: false,
            active_tools: &[],
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
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: Some(&intake),
            plan_review: None,
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
                .any(|line| line.contains("Planning choices"))
        );
        assert!(rendered.iter().any(|line| line.contains("● Stack")));
        assert!(rendered.iter().any(|line| line.contains("Какой stack?")));
        assert!(
            rendered
                .iter()
                .any(|line| line.starts_with("› 1. Telegram Bot API"))
        );
        assert!(rendered.iter().any(|line| line.contains("Type something")));
        assert!(rendered.iter().any(|line| line.contains("Chat about this")));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("Skip interview and plan immediately"))
        );
    }

    #[test]
    fn plan_intake_renders_multi_select_checkboxes() {
        let intake = PlanIntakeState::from_metadata(&json!({
            "plan_intake": {
                "id": "shop",
                "title": "Shop",
                "questions": [{
                    "id": "features",
                    "header": "Features",
                    "prompt": "Какие функции?",
                    "multi_select": true,
                    "options": [
                        {"id": "catalog", "label": "Catalog"},
                        {"id": "cart", "label": "Cart"}
                    ],
                    "allow_custom": true
                }]
            }
        }))
        .expect("intake");
        let state = VisualState {
            model: "test/model",
            permission_mode: "plan",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "planning choices",
            pending_approval: None,
            pending_model: false,
            active_tools: &[],
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
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: Some(&intake),
            plan_review: None,
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
                .any(|line| line.starts_with("› [x] 1. Catalog"))
        );
        assert!(rendered.iter().any(|line| line.contains("[ ] 2. Cart")));
        assert!(rendered.iter().any(|line| line.contains("Space toggle")));
    }

    #[test]
    fn typing_keeps_idle_composer_row_stable() {
        fn idle_state(input: &'static str) -> VisualState<'static> {
            VisualState {
                model: "test/model",
                permission_mode: "normal",
                cwd: Path::new("/tmp/workspace"),
                session_label: "1",
                input,
                input_paste_ranges: &[],
                footer: "enter send",
                status: "ready",
                pending_approval: None,
                pending_model: false,
                active_tools: &[],
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
                config_summary: None,
                config_summary_scroll: 0,
                plan_intake: None,
                plan_review: None,
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
            permission_mode: "normal",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "",
            status: "calling model...",
            pending_approval: None,
            pending_model: true,
            active_tools: &[],
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
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: None,
            plan_review: None,
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
            permission_mode: "normal",
            cwd: Path::new("/tmp/workspace"),
            session_label: "1",
            input: "",
            input_paste_ranges: &[],
            footer: "enter send",
            status: "done · 7s",
            pending_approval: None,
            pending_model: false,
            active_tools: &[],
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
            config_summary: None,
            config_summary_scroll: 0,
            plan_intake: None,
            plan_review: None,
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
