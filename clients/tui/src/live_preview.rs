use ratatui::{style::Style, text::Line};

use crate::visual::{
    VisualMessage, VisualRole, append_reasoning_preview_lines, muted_style,
    reasoning_preview_visible, tool_action_body, tool_output_prefix_style, tool_output_style,
    tool_status_style,
};

pub(crate) fn live_preview_height(state: &crate::visual::VisualState<'_>, available: u16) -> u16 {
    if available == 0 || state.pending_approval.is_some() {
        return 0;
    }
    if state.streaming || reasoning_preview_visible(state) {
        return available.max(1);
    }
    0
}

pub(crate) fn live_preview_lines(
    state: &crate::visual::VisualState<'_>,
    width: usize,
    max_lines: usize,
) -> Vec<Line<'static>> {
    let mut body = Vec::new();
    append_reasoning_preview_lines(&mut body, state, width);
    if let Some(message) = state.streaming_message {
        if !body.is_empty() {
            body.push(Line::raw(""));
        }
        append_live_preview_message_lines(&mut body, message, width);
        return tail_lines(body, max_lines.max(1));
    }

    tail_lines(body, max_lines.max(1))
}

fn append_live_preview_message_lines(
    lines: &mut Vec<Line<'static>>,
    message: &VisualMessage,
    width: usize,
) {
    if matches!(message.role, VisualRole::Tool) {
        if let Some(card) = &message.tool {
            append_tool_preview_lines(lines, card, width);
        }
        return;
    }

    let (prefix, style) = match message.role {
        VisualRole::User => ("› ", Style::default().fg(ratatui::style::Color::Cyan)),
        VisualRole::Assistant => ("• ", Style::default().fg(ratatui::style::Color::Reset)),
        VisualRole::Draft => ("◦ draft ", muted_style()),
        VisualRole::System => ("  ", muted_style()),
        VisualRole::Error => ("! ", Style::default().fg(ratatui::style::Color::Red)),
        VisualRole::Tool => ("  ", muted_style()),
    };
    if message.text.is_empty() {
        lines.push(Line::from(ratatui::text::Span::styled(
            prefix.trim_end().to_owned(),
            style,
        )));
        return;
    }

    if matches!(message.role, VisualRole::Assistant | VisualRole::Draft) {
        let (stable_text, tail_text) = split_live_markdown_text(&message.text);
        if !stable_text.is_empty() {
            lines.extend(crate::markdown::render_assistant_markdown(
                stable_text,
                prefix,
                style,
                width,
            ));
        }
        if !tail_text.is_empty() {
            append_plain_preview_text(
                lines,
                tail_text,
                if stable_text.is_empty() { prefix } else { "  " },
                style,
                width,
            );
        }
        return;
    }

    append_plain_preview_text(lines, &message.text, prefix, style, width);
}

fn append_tool_preview_lines(
    lines: &mut Vec<Line<'static>>,
    card: &crate::visual::ToolCard,
    width: usize,
) {
    let status = tool_status_style(card.status);
    let action = tool_action_body(card, status.label);
    lines.push(Line::from(vec![
        ratatui::text::Span::styled(format!("{} ", status.marker), status.marker_style),
        ratatui::text::Span::styled(status.label.to_owned(), status.label_style),
        ratatui::text::Span::styled(" ", muted_style()),
        ratatui::text::Span::styled(action, status.action_style),
    ]));

    if !card.output_preview.is_empty() {
        let preview_width = width.saturating_sub(4).max(1);
        let mut first_output_line = true;
        for raw in card.output_preview.lines() {
            for segment in crate::visual::wrap_text(raw, preview_width) {
                let prefix = if first_output_line { "  └ " } else { "    " };
                lines.push(Line::from(vec![
                    ratatui::text::Span::styled(prefix, tool_output_prefix_style(card.status)),
                    ratatui::text::Span::styled(segment, tool_output_style(card.status)),
                ]));
                first_output_line = false;
            }
        }
    }
}

fn split_live_markdown_text(text: &str) -> (&str, &str) {
    if text.ends_with('\n') {
        return (text, "");
    }
    text.rfind('\n')
        .map(|index| text.split_at(index + 1))
        .unwrap_or(("", text))
}

pub(crate) fn append_plain_preview_text(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    prefix: &str,
    style: Style,
    width: usize,
) {
    let mut first_segment = true;
    let text_width = width.saturating_sub(prefix.chars().count()).max(1);
    for source_line in text.lines() {
        let segments = crate::visual::wrap_text(source_line, text_width);
        if segments.is_empty() {
            lines.push(Line::raw(""));
            continue;
        }

        for segment in segments {
            let line_prefix = if first_segment { prefix } else { "  " };
            lines.push(Line::from(vec![
                ratatui::text::Span::styled(line_prefix.to_owned(), style),
                ratatui::text::Span::styled(segment, style),
            ]));
            first_segment = false;
        }
    }
}

fn tail_lines(mut lines: Vec<Line<'static>>, limit: usize) -> Vec<Line<'static>> {
    if lines.len() <= limit {
        return lines;
    }
    lines.drain(0..lines.len() - limit);
    lines
}
