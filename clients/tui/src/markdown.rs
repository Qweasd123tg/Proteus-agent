use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub(crate) fn render_assistant_markdown(
    text: &str,
    prefix: &str,
    prefix_style: Style,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut first = true;
    let mut in_code_block = false;
    let content_width = width.saturating_sub(prefix.chars().count()).max(1);

    if text.is_empty() {
        lines.push(Line::from(Span::styled(
            prefix.trim_end().to_owned(),
            prefix_style,
        )));
        return lines;
    }

    for raw_line in text.lines() {
        let source = raw_line.trim_end_matches('\r');
        let trimmed = source.trim_start();

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            let label = trimmed.trim_start_matches("```").trim();
            let display = if label.is_empty() {
                "```".to_owned()
            } else {
                format!("``` {label}")
            };
            push_line(
                &mut lines,
                &mut first,
                prefix,
                prefix_style,
                vec![Span::styled(display, code_fence_style())],
            );
            continue;
        }

        if source.is_empty() {
            lines.push(Line::raw(""));
            continue;
        }

        if in_code_block {
            for segment in wrap_text(source, content_width) {
                push_line(
                    &mut lines,
                    &mut first,
                    prefix,
                    prefix_style,
                    vec![
                        Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                        Span::styled(segment, code_style()),
                    ],
                );
            }
            continue;
        }

        if let Some((level, heading)) = heading(source) {
            let marker = "#".repeat(level);
            for segment in wrap_text(heading, content_width.saturating_sub(level + 1).max(1)) {
                push_line(
                    &mut lines,
                    &mut first,
                    prefix,
                    prefix_style,
                    vec![
                        Span::styled(format!("{marker} "), heading_marker_style()),
                        Span::styled(segment, heading_style()),
                    ],
                );
            }
            continue;
        }

        if let Some(quote) = trimmed.strip_prefix("> ") {
            for segment in wrap_text(quote, content_width.saturating_sub(2).max(1)) {
                let mut spans = vec![Span::styled("│ ", Style::default().fg(Color::DarkGray))];
                spans.extend(inline_spans(&segment, quote_style()));
                push_line(&mut lines, &mut first, prefix, prefix_style, spans);
            }
            continue;
        }

        if let Some((marker, body)) = list_item(trimmed) {
            let marker_width = marker.chars().count() + 1;
            for (idx, segment) in wrap_text(body, content_width.saturating_sub(marker_width).max(1))
                .into_iter()
                .enumerate()
            {
                let mut spans = if idx == 0 {
                    vec![
                        Span::styled(marker.to_owned(), list_marker_style()),
                        Span::raw(" "),
                    ]
                } else {
                    vec![Span::raw(" ".repeat(marker_width))]
                };
                spans.extend(inline_spans(&segment, Style::default()));
                push_line(&mut lines, &mut first, prefix, prefix_style, spans);
            }
            continue;
        }

        for segment in wrap_text(source, content_width) {
            let spans = inline_spans(&segment, Style::default());
            push_line(&mut lines, &mut first, prefix, prefix_style, spans);
        }
    }

    lines
}

fn push_line(
    lines: &mut Vec<Line<'static>>,
    first: &mut bool,
    prefix: &str,
    prefix_style: Style,
    content: Vec<Span<'static>>,
) {
    let line_prefix = if *first { prefix } else { "  " };
    let mut spans = Vec::with_capacity(content.len() + 1);
    spans.push(Span::styled(line_prefix.to_owned(), prefix_style));
    spans.extend(content);
    lines.push(Line::from(spans));
    *first = false;
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = trimmed.get(level..)?;
    Some((level, rest.strip_prefix(' ')?.trim()))
}

fn list_item(line: &str) -> Option<(&str, &str)> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(body) = line.strip_prefix(marker) {
            return Some((marker.trim(), body));
        }
    }

    let dot = line.find(". ")?;
    if dot == 0 || !line[..dot].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((&line[..=dot], &line[dot + 2..]))
}

fn inline_spans(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;

    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            spans.push(Span::styled(after[..end].to_owned(), inline_code_style()));
            rest = &after[end + 1..];
            continue;
        }

        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            spans.push(Span::styled(
                after[..end].to_owned(),
                base.add_modifier(Modifier::BOLD),
            ));
            rest = &after[end + 2..];
            continue;
        }

        if let Some(after) = rest.strip_prefix('*')
            && let Some(end) = after.find('*')
        {
            spans.push(Span::styled(
                after[..end].to_owned(),
                base.add_modifier(Modifier::ITALIC),
            ));
            rest = &after[end + 1..];
            continue;
        }

        let next = next_markup(rest).unwrap_or(rest.len());
        spans.push(Span::styled(rest[..next].to_owned(), base));
        rest = &rest[next..];
    }

    spans
}

fn next_markup(text: &str) -> Option<usize> {
    ["`", "**", "*"]
        .into_iter()
        .filter_map(|needle| text.find(needle))
        .filter(|index| *index > 0)
        .min()
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
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

fn heading_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn heading_marker_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn list_marker_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn quote_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn code_style() -> Style {
    Style::default().fg(Color::Green)
}

fn code_fence_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn inline_code_style() -> Style {
    Style::default().fg(Color::Yellow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_heading_and_inline_code() {
        let lines =
            render_assistant_markdown("# Title\nUse `cargo test`.", "• ", Style::default(), 80);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content.as_ref(), "• ");
        assert_eq!(lines[0].spans[2].content.as_ref(), "Title");
        assert_eq!(lines[1].spans[2].content.as_ref(), "cargo test");
    }

    #[test]
    fn prefixes_only_first_line() {
        let lines = render_assistant_markdown("- first\n- second", "• ", Style::default(), 80);
        assert_eq!(lines[0].spans[0].content.as_ref(), "• ");
        assert_eq!(lines[1].spans[0].content.as_ref(), "  ");
    }
}
