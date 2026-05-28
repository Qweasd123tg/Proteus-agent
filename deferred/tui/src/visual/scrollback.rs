use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use super::{
    InputPasteRange, display_segments_from_paste_ranges, muted_style, wrap_segments_for_width,
};

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
    pub call_id: proteus_contracts::domain::CallId,
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

pub(crate) fn render_tool_card_lines(card: &ToolCard, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    append_tool_card_lines(&mut lines, card, width);
    lines
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
            marker: crate::motion::running_tool_marker(),
            marker_style: crate::motion::running_tool_marker_style(),
            label: "running",
            label_style: Style::default().fg(Color::Rgb(255, 149, 0)),
            action_style: Style::default().fg(Color::LightCyan),
        },
        ToolStatus::Ok => ToolStatusStyle {
            marker: "●",
            marker_style: Style::default().fg(Color::Green),
            label: "ran",
            label_style: Style::default().fg(Color::Green),
            action_style: Style::default().fg(Color::LightCyan),
        },
        ToolStatus::Err => ToolStatusStyle {
            marker: "●",
            marker_style: Style::default().fg(Color::Red),
            label: "failed",
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
