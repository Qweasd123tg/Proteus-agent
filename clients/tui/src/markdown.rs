use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(crate) fn render_assistant_markdown(
    text: &str,
    prefix: &str,
    prefix_style: Style,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut first = true;
    let mut in_code_block = false;
    let content_width = width.saturating_sub(display_width(prefix)).max(1);

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

        if in_code_block {
            let segments = if source.is_empty() {
                vec![String::new()]
            } else {
                wrap_text(source, content_width)
            };
            for segment in segments {
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

        if source.is_empty() {
            lines.push(Line::raw(""));
            index += 1;
            continue;
        }

        if is_horizontal_rule(trimmed) {
            push_line(
                &mut lines,
                &mut first,
                prefix,
                prefix_style,
                vec![Span::styled(
                    "─".repeat(content_width.min(72)),
                    horizontal_rule_style(),
                )],
            );
            index += 1;
            continue;
        }

        if let Some((table, consumed)) = parse_table(&raw_lines, index, content_width) {
            render_table_lines(&mut lines, &mut first, prefix, prefix_style, &table);
            index += consumed;
            continue;
        }

        if let Some((_level, heading)) = heading(source) {
            for (idx, segment) in wrap_text(heading, content_width.saturating_sub(2).max(1))
                .into_iter()
                .enumerate()
            {
                push_line(
                    &mut lines,
                    &mut first,
                    prefix,
                    prefix_style,
                    heading_spans(&segment, idx == 0),
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
            let marker_width = display_width(marker) + 1;
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
        .map(|cell| inline_width(cell))
        .collect::<Vec<_>>();
    for row in &rows {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(inline_width(cell));
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
    for row_line in render_table_row_lines(&table.header, &table.widths, true) {
        push_line(lines, first, prefix, prefix_style, row_line);
    }
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
        for row_line in render_table_row_lines(row, &table.widths, false) {
            push_line(lines, first, prefix, prefix_style, row_line);
        }
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

fn render_table_row_lines(
    row: &[String],
    widths: &[usize],
    header: bool,
) -> Vec<Vec<Span<'static>>> {
    let wrapped_cells = widths
        .iter()
        .enumerate()
        .map(|(idx, width)| {
            let cell = row.get(idx).map(String::as_str).unwrap_or_default();
            let base_style = if header {
                table_header_style()
            } else {
                Style::default()
            };
            wrap_spans(inline_spans(cell, base_style), *width)
        })
        .collect::<Vec<_>>();
    let height = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let mut lines = Vec::with_capacity(height);

    for line_idx in 0..height {
        let mut spans = vec![Span::styled("│", table_border_style())];
        for (idx, width) in widths.iter().enumerate() {
            let empty = Vec::new();
            let cell_spans = wrapped_cells
                .get(idx)
                .and_then(|lines| lines.get(line_idx))
                .unwrap_or(&empty);
            let cell_width = spans_width(cell_spans);
            let padding = width.saturating_sub(cell_width);
            spans.push(Span::raw(" "));
            spans.extend(cell_spans.clone());
            spans.push(Span::raw(" ".repeat(padding + 1)));
            spans.push(Span::styled("│", table_border_style()));
        }
        lines.push(spans);
    }

    lines
}

fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    if spans.is_empty() {
        return vec![Vec::new()];
    }

    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut current_width = 0usize;

    for span in spans {
        for ch in span.content.chars() {
            let ch_width = ch.width().unwrap_or(0);
            if current_width > 0 && current_width + ch_width > width {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
            }
            push_text_span(&mut current, ch.to_string(), span.style);
            current_width += ch_width;
            if current_width >= width {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
            }
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn push_text_span(spans: &mut Vec<Span<'static>>, text: String, style: Style) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.style == style
    {
        last.content.to_mut().push_str(&text);
        return;
    }
    spans.push(Span::styled(text, style));
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

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }
    trimmed
        .chars()
        .all(|ch| ch == '-' || ch == '_' || ch == '*')
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
            spans.push(Span::styled(
                after[..end].to_owned(),
                inline_code_style().add_modifier(base.add_modifier),
            ));
            rest = &after[end + 1..];
            continue;
        }

        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            spans.extend(inline_spans(
                &after[..end],
                base.add_modifier(Modifier::BOLD),
            ));
            rest = &after[end + 2..];
            continue;
        }

        if let Some(after) = rest.strip_prefix('*')
            && let Some(end) = after.find('*')
        {
            spans.extend(inline_spans(
                &after[..end],
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
    let mut segment_width = 0usize;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if segment_width > 0 && segment_width + ch_width > width {
            segments.push(std::mem::take(&mut segment));
            segment_width = 0;
        }
        segment.push(ch);
        segment_width += ch_width;
        if segment_width >= width {
            segments.push(std::mem::take(&mut segment));
            segment_width = 0;
        }
    }
    if !segment.is_empty() {
        segments.push(segment);
    }
    segments
}

fn heading_spans(text: &str, first: bool) -> Vec<Span<'static>> {
    let marker = if first { "▌ " } else { "  " };
    vec![
        Span::styled(marker, heading_marker_style()),
        Span::styled(text.to_owned(), heading_style()),
    ]
}

fn inline_width(text: &str) -> usize {
    spans_width(&inline_spans(text, Style::default()))
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| display_width(span.content.as_ref()))
        .sum()
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn heading_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn heading_marker_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn horizontal_rule_style() -> Style {
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
    fn wraps_table_cells_to_available_width() {
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
        assert!(lines.iter().all(|line| line.width() <= 24));
        for ch in "abcdefghijk123456789".chars() {
            assert!(rendered.contains(ch));
        }
    }

    #[test]
    fn table_cells_render_inline_markdown() {
        let lines = render_assistant_markdown(
            "| Provider | Что делает |\n| --- | --- |\n| **`type_signatures`** | Вытаскивает сигнатуры |",
            "• ",
            Style::default(),
            80,
        );
        let rendered = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("type_signatures"));
        assert!(!rendered.contains("**"));
        assert!(!rendered.contains('`'));
        assert_eq!(lines[3].spans[3].style.fg, Some(Color::Yellow));
        assert!(
            lines[3].spans[3]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn table_width_uses_terminal_columns() {
        let lines = render_assistant_markdown(
            "| Provider | Что делает |\n| --- | --- |\n| **`type_signatures`** | Вытаскивает сигнатуры функций/типов, к которым обращается задача |\n| **`open_issues`** | Поиск по issues/багам, связанным с задачей |",
            "• ",
            Style::default(),
            58,
        );

        assert!(lines.iter().all(|line| line.width() <= 58));
        assert!(lines[0].spans[1].content.ends_with('┐'));
        assert_eq!(lines[3].spans.last().unwrap().content.as_ref(), "│");
    }

    #[test]
    fn renders_horizontal_rules_without_raw_markers() {
        let lines = render_assistant_markdown("---\n## Section", "• ", Style::default(), 30);

        assert_eq!(
            lines[0].spans[1].content.as_ref(),
            "────────────────────────────"
        );
        assert_eq!(lines[1].spans[1].content.as_ref(), "▌ ");
        assert_eq!(lines[1].spans[2].content.as_ref(), "Section");
    }

    #[test]
    fn keeps_fenced_code_blocks_contiguous() {
        let lines = render_assistant_markdown(
            "``` toml\n[modules]\ncontext = \"spec_first\"\n\n[module_config.context.spec_first]\n```",
            "• ",
            Style::default(),
            80,
        );
        assert_eq!(lines[0].spans[1].content.as_ref(), "``` toml");
        assert_eq!(lines[1].spans[1].content.as_ref(), "│ ");
        assert_eq!(lines[2].spans[1].content.as_ref(), "│ ");
        assert_eq!(lines[3].spans[1].content.as_ref(), "│ ");
        assert_eq!(lines[3].spans[2].content.as_ref(), "");
        assert_eq!(lines[4].spans[1].content.as_ref(), "│ ");
        assert_eq!(lines[5].spans[1].content.as_ref(), "```");
    }
}
