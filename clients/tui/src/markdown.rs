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

    let raw_lines = text.lines().collect::<Vec<_>>();
    let mut index = 0usize;
    while let Some(raw_line) = raw_lines.get(index) {
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
            index += 1;
            continue;
        }

        if source.is_empty() {
            lines.push(Line::raw(""));
            index += 1;
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
            index += 1;
            continue;
        }

        if let Some((table, consumed)) = parse_table(&raw_lines, index, content_width) {
            render_table_lines(&mut lines, &mut first, prefix, prefix_style, &table);
            index += consumed;
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
            index += 1;
            continue;
        }

        if let Some(quote) = trimmed.strip_prefix("> ") {
            for segment in wrap_text(quote, content_width.saturating_sub(2).max(1)) {
                let mut spans = vec![Span::styled("│ ", Style::default().fg(Color::DarkGray))];
                spans.extend(inline_spans(&segment, quote_style()));
                push_line(&mut lines, &mut first, prefix, prefix_style, spans);
            }
            index += 1;
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
            index += 1;
            continue;
        }

        for segment in wrap_text(source, content_width) {
            let spans = inline_spans(&segment, Style::default());
            push_line(&mut lines, &mut first, prefix, prefix_style, spans);
        }
        index += 1;
    }

    lines
}

#[derive(Debug)]
struct MarkdownTable {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
    widths: Vec<usize>,
}

fn parse_table(lines: &[&str], start: usize, max_width: usize) -> Option<(MarkdownTable, usize)> {
    let header = parse_pipe_row(lines.get(start)?.trim_end_matches('\r'))?;
    let separator = parse_pipe_row(lines.get(start + 1)?.trim_end_matches('\r'))?;
    if header.len() < 2 || separator.len() != header.len() {
        return None;
    }
    if !separator.iter().all(|cell| is_separator_cell(cell)) {
        return None;
    }

    let mut rows = Vec::new();
    let mut index = start + 2;
    while let Some(raw_line) = lines.get(index) {
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() {
            break;
        }
        let Some(row) = parse_pipe_row(line) else {
            break;
        };
        if row.len() < 2 {
            break;
        }
        rows.push(normalize_row(row, header.len()));
        index += 1;
    }

    let mut widths = header
        .iter()
        .map(|cell| cell.chars().count())
        .collect::<Vec<_>>();
    for row in &rows {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(cell.chars().count());
        }
    }
    fit_table_width(&mut widths, max_width);

    Some((
        MarkdownTable {
            header,
            rows,
            widths,
        },
        index - start,
    ))
}

fn parse_pipe_row(line: &str) -> Option<Vec<String>> {
    if !line.contains('|') {
        return None;
    }
    let trimmed = line.trim();
    let inner = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .strip_suffix('|')
        .unwrap_or(trimmed.strip_prefix('|').unwrap_or(trimmed));
    let cells = inner
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect::<Vec<_>>();
    (cells.len() >= 2).then_some(cells)
}

fn is_separator_cell(cell: &str) -> bool {
    let trimmed = cell.trim();
    trimmed.len() >= 3 && trimmed.contains('-') && trimmed.chars().all(|ch| ch == '-' || ch == ':')
}

fn normalize_row(mut row: Vec<String>, width: usize) -> Vec<String> {
    row.truncate(width);
    row.resize_with(width, String::new);
    row
}

fn fit_table_width(widths: &mut [usize], max_width: usize) {
    if widths.is_empty() {
        return;
    }
    let overhead = widths.len() + 1 + widths.len() * 2;
    let available = max_width.saturating_sub(overhead).max(widths.len());
    while widths.iter().sum::<usize>() > available {
        let Some((idx, width)) = widths.iter().enumerate().max_by_key(|(_, width)| **width) else {
            return;
        };
        if *width <= 4 {
            return;
        }
        widths[idx] -= 1;
    }
}

fn render_table_lines(
    lines: &mut Vec<Line<'static>>,
    first: &mut bool,
    prefix: &str,
    prefix_style: Style,
    table: &MarkdownTable,
) {
    push_line(
        lines,
        first,
        prefix,
        prefix_style,
        vec![Span::styled(
            table_border('┌', '┬', '┐', &table.widths),
            table_border_style(),
        )],
    );
    push_line(
        lines,
        first,
        prefix,
        prefix_style,
        render_table_row(&table.header, &table.widths, true),
    );
    push_line(
        lines,
        first,
        prefix,
        prefix_style,
        vec![Span::styled(
            table_border('├', '┼', '┤', &table.widths),
            table_border_style(),
        )],
    );
    for row in &table.rows {
        push_line(
            lines,
            first,
            prefix,
            prefix_style,
            render_table_row(row, &table.widths, false),
        );
    }
    push_line(
        lines,
        first,
        prefix,
        prefix_style,
        vec![Span::styled(
            table_border('└', '┴', '┘', &table.widths),
            table_border_style(),
        )],
    );
}

fn table_border(left: char, middle: char, right: char, widths: &[usize]) -> String {
    let mut out = String::new();
    out.push(left);
    for (idx, width) in widths.iter().enumerate() {
        if idx > 0 {
            out.push(middle);
        }
        out.push_str(&"─".repeat(width + 2));
    }
    out.push(right);
    out
}

fn render_table_row(row: &[String], widths: &[usize], header: bool) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled("│", table_border_style())];
    for (idx, width) in widths.iter().enumerate() {
        let cell = row.get(idx).map(String::as_str).unwrap_or_default();
        let cell = truncate_cell(cell, *width);
        let padding = width.saturating_sub(cell.chars().count());
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            cell,
            if header {
                table_header_style()
            } else {
                Style::default()
            },
        ));
        spans.push(Span::raw(" ".repeat(padding + 1)));
        spans.push(Span::styled("│", table_border_style()));
    }
    spans
}

fn truncate_cell(cell: &str, width: usize) -> String {
    if cell.chars().count() <= width {
        return cell.to_owned();
    }
    if width <= 1 {
        return "…".to_owned();
    }
    let prefix = cell.chars().take(width - 1).collect::<String>();
    format!("{prefix}…")
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

fn table_border_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn table_header_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
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

    #[test]
    fn renders_pipe_table_as_aligned_rows() {
        let lines = render_assistant_markdown(
            "| Name | Value |\n| --- | --- |\n| alpha | 1 |\n| beta | 20 |",
            "• ",
            Style::default(),
            80,
        );
        assert_eq!(lines[0].spans[0].content.as_ref(), "• ");
        assert_eq!(lines[0].spans[1].content.as_ref(), "┌───────┬───────┐");
        assert_eq!(lines[1].spans[3].content.as_ref(), "Name");
        assert_eq!(lines[3].spans[3].content.as_ref(), "alpha");
        assert_eq!(lines[4].spans[3].content.as_ref(), "beta");
    }

    #[test]
    fn truncates_table_cells_to_available_width() {
        let lines = render_assistant_markdown(
            "| Long | Also |\n| --- | --- |\n| abcdefghijk | 123456789 |",
            "• ",
            Style::default(),
            24,
        );
        let rendered = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(rendered.contains('…'));
    }
}
