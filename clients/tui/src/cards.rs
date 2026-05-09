use agent_contracts::app_protocol::AppApprovalRequest;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::visual::{
    ToolCard, ToolStatus, VisualState, compact_value, display_path, muted_style, tool_action_text,
    truncate, wrap_text,
};

pub(crate) fn render_scrollback_header(
    state: &VisualState<'_>,
    width: usize,
) -> Vec<Line<'static>> {
    session_card(state, width)
}

pub(crate) fn append_approval_lines(
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
        Span::styled("? ", muted_style()),
        Span::styled(
            "Would you like to allow this tool call?",
            Style::default().fg(Color::Reset),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  tool: "),
        Span::styled(request.call.name.clone(), Style::default().fg(Color::Reset)),
        Span::styled(format!(" · {safety}"), muted_style()),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  cwd:  "),
        Span::styled(
            truncate(request.cwd.display().to_string(), text_width),
            muted_style(),
        ),
    ]));
    for seg in wrap_text(&format!("reason: {}", request.reason), text_width) {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(seg, muted_style()),
        ]));
    }
    for seg in wrap_text(&format!("args: {args}"), text_width) {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(seg, muted_style()),
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

#[allow(dead_code)]
fn _tool_card_preview_lines(lines: &mut Vec<Line<'static>>, card: &ToolCard, width: usize) {
    let (marker, marker_style, label, label_style) = match card.status {
        ToolStatus::Running => ("•", muted_style(), "Running", muted_style()),
        ToolStatus::Ok => ("•", muted_style(), "Ran", Style::default().fg(Color::Reset)),
        ToolStatus::Err => (
            "✗",
            Style::default().fg(Color::Red),
            "Failed",
            Style::default().fg(Color::Red),
        ),
    };
    let action = tool_action_text(card, label);
    lines.push(Line::from(vec![
        Span::styled(format!("{marker} "), marker_style),
        Span::styled(action, label_style),
    ]));

    if !card.output_preview.is_empty() {
        let preview_width = width.saturating_sub(4).max(1);
        let mut first_output_line = true;
        for raw in card.output_preview.lines() {
            for segment in wrap_text(raw, preview_width) {
                let prefix = if first_output_line { "  └ " } else { "    " };
                lines.push(Line::from(vec![
                    Span::styled(prefix, muted_style()),
                    Span::styled(segment, muted_style()),
                ]));
                first_output_line = false;
            }
        }
    }
}
