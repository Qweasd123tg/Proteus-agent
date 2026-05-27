use agent_contracts::{app_protocol::AppApprovalRequest, domain::ToolSafety};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::visual::{
    ToolCard, VisualState, compact_value, display_path, muted_style, tool_action_body,
    tool_output_prefix_style, tool_output_style, tool_status_style, truncate, wrap_text,
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
    let (remember_label, remember_hint) = approval_remember_text(request);
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("1. Yes, proceed", Style::default().fg(Color::Green)),
        Span::raw("  "),
        Span::styled(remember_label, Style::default().fg(Color::Green)),
        Span::raw("  "),
        Span::styled("3. No", Style::default().fg(Color::Red)),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(remember_hint, Style::default().fg(Color::DarkGray)),
    ]));
}

fn approval_remember_text(request: &AppApprovalRequest) -> (&'static str, &'static str) {
    let safety = request.tool_spec.as_ref().map(|spec| &spec.safety);
    if request.call.name == "shell"
        || matches!(
            safety,
            Some(ToolSafety::RunsCommands | ToolSafety::Network | ToolSafety::Dangerous)
        )
    {
        (
            "2. Yes, remember exact call",
            "y/н approve · p/з remember exact call · n/т/esc deny",
        )
    } else {
        (
            "2. Yes, remember tool in cwd",
            "y/н approve · p/з remember · n/т/esc deny",
        )
    }
}

pub(crate) fn footer_plain_line(state: &VisualState<'_>, width: usize) -> String {
    let left = footer_left_text(state);
    if state.pending_model || state.pending_approval.is_some() {
        truncate(format!("  {left}"), width)
    } else {
        truncate(format!("  {left}    {}", state.footer), width)
    }
}

pub(crate) fn footer_left_text(state: &VisualState<'_>) -> String {
    if let Some(request) = state.pending_approval {
        let (_, hint) = approval_remember_text(request);
        hint.replace(" · n/т/esc deny", " · 3/n/esc deny")
    } else if state.resume_picker.is_some() {
        "type search · enter resume · esc close · up/down select".to_owned()
    } else if state.config_summary.is_some() {
        "configs · esc close · up/down scroll".to_owned()
    } else if !crate::slash_commands::matching_slash_commands(state.input).is_empty() {
        "enter/tab complete · up/down select · enter run exact".to_owned()
    } else if state.pending_model {
        "turn running · esc cancel".to_owned()
    } else if let Some(done) = state.status.strip_prefix("done") {
        format!("✓ done{} · enter send", done)
    } else {
        state.status.to_owned()
    }
}

fn session_card(state: &VisualState<'_>, width: usize) -> Vec<Line<'static>> {
    let cwd = display_path(state.cwd);
    let rows = [
        format!("model:     {}", state.model),
        format!("mode:      {}", state.permission_mode),
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
