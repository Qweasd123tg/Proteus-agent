const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const ITALIC: &str = "\x1b[3m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";

pub(crate) fn render_markdown_ansi(text: &str) -> String {
    let mut rendered = String::new();
    let mut in_code_block = false;
    let lines = text.lines().collect::<Vec<_>>();
    let mut index = 0usize;

    while let Some(raw_line) = lines.get(index) {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            let lang = trimmed.trim_start_matches("```").trim();
            if lang.is_empty() {
                rendered.push_str(DIM);
                rendered.push_str("```");
                rendered.push_str(RESET);
            } else {
                rendered.push_str(DIM);
                rendered.push_str("``` ");
                rendered.push_str(lang);
                rendered.push_str(RESET);
            }
            rendered.push('\n');
            index += 1;
            continue;
        }

        if in_code_block {
            rendered.push_str(DIM);
            rendered.push_str(line);
            rendered.push_str(RESET);
            rendered.push('\n');
            index += 1;
            continue;
        }

        if let Some((table, consumed)) = parse_table(&lines, index) {
            rendered.push_str(&render_table_ansi(&table));
            index += consumed;
            continue;
        }

        if let Some((level, heading)) = heading(line) {
            let marker = "#".repeat(level);
            rendered.push_str(CYAN);
            rendered.push_str(BOLD);
            rendered.push_str(&marker);
            rendered.push(' ');
            rendered.push_str(heading);
            rendered.push_str(RESET);
            rendered.push('\n');
            index += 1;
            continue;
        }

        if let Some(quote) = trimmed.strip_prefix("> ") {
            rendered.push_str(DIM);
            rendered.push_str("│ ");
            rendered.push_str(RESET);
            rendered.push_str(&render_inline_ansi(quote));
            rendered.push('\n');
            index += 1;
            continue;
        }

        if let Some((marker, body)) = list_item(trimmed) {
            rendered.push_str(YELLOW);
            rendered.push_str(marker);
            rendered.push_str(RESET);
            rendered.push(' ');
            rendered.push_str(&render_inline_ansi(body));
            rendered.push('\n');
            index += 1;
            continue;
        }

        rendered.push_str(&render_inline_ansi(line));
        rendered.push('\n');
        index += 1;
    }

    if text.ends_with('\n') {
        rendered
    } else {
        rendered.pop();
        rendered
    }
}

#[derive(Debug)]
struct MarkdownTable {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
    widths: Vec<usize>,
}

fn parse_table(lines: &[&str], start: usize) -> Option<(MarkdownTable, usize)> {
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

fn render_table_ansi(table: &MarkdownTable) -> String {
    let mut out = String::new();
    out.push_str(DIM);
    out.push_str(&table_border('┌', '┬', '┐', &table.widths));
    out.push_str(RESET);
    out.push('\n');
    out.push_str(&render_table_row(&table.header, &table.widths, true));
    out.push('\n');
    out.push_str(DIM);
    out.push_str(&table_border('├', '┼', '┤', &table.widths));
    out.push_str(RESET);
    out.push('\n');
    for row in &table.rows {
        out.push_str(&render_table_row(row, &table.widths, false));
        out.push('\n');
    }
    out.push_str(DIM);
    out.push_str(&table_border('└', '┴', '┘', &table.widths));
    out.push_str(RESET);
    out.push('\n');
    out
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

fn render_table_row(row: &[String], widths: &[usize], header: bool) -> String {
    let mut out = String::new();
    out.push_str(DIM);
    out.push('│');
    out.push_str(RESET);
    for (idx, width) in widths.iter().enumerate() {
        let cell = row.get(idx).map(String::as_str).unwrap_or_default();
        let padding = width.saturating_sub(cell.chars().count());
        out.push(' ');
        if header {
            out.push_str(BOLD);
            out.push_str(cell);
            out.push_str(RESET);
        } else {
            out.push_str(cell);
        }
        out.push_str(&" ".repeat(padding + 1));
        out.push_str(DIM);
        out.push('│');
        out.push_str(RESET);
    }
    out
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = trimmed.get(level..)?;
    let heading = rest.strip_prefix(' ')?;
    Some((level, heading.trim()))
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

fn render_inline_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            out.push_str(YELLOW);
            out.push_str(&after[..end]);
            out.push_str(RESET);
            rest = &after[end + 1..];
            continue;
        }

        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            out.push_str(BOLD);
            out.push_str(&after[..end]);
            out.push_str(RESET);
            rest = &after[end + 2..];
            continue;
        }

        if let Some(after) = rest.strip_prefix('*')
            && let Some(end) = after.find('*')
        {
            out.push_str(ITALIC);
            out.push_str(&after[..end]);
            out.push_str(RESET);
            rest = &after[end + 1..];
            continue;
        }

        let next = next_markup(rest).unwrap_or(rest.len());
        out.push_str(&rest[..next]);
        rest = &rest[next..];
    }

    out
}

fn next_markup(text: &str) -> Option<usize> {
    ["`", "**", "*"]
        .into_iter()
        .filter_map(|needle| text.find(needle))
        .filter(|index| *index > 0)
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_headings_and_inline_code() {
        let rendered = render_markdown_ansi("# Title\nUse `cargo test`.");
        assert!(rendered.contains("\x1b[36m\x1b[1m# Title\x1b[0m"));
        assert!(rendered.contains("Use \x1b[33mcargo test\x1b[0m."));
    }

    #[test]
    fn preserves_trailing_newline() {
        assert!(render_markdown_ansi("hello\n").ends_with('\n'));
        assert!(!render_markdown_ansi("hello").ends_with('\n'));
    }

    #[test]
    fn renders_pipe_table_as_aligned_box() {
        let rendered =
            render_markdown_ansi("| Name | Value |\n| --- | --- |\n| alpha | 1 |\n| beta | 20 |");
        assert!(rendered.contains("┌───────┬───────┐"));
        assert!(rendered.contains("\x1b[1mName\x1b[0m"));
        assert!(rendered.contains("alpha"));
        assert!(rendered.contains("beta "));
        assert!(rendered.contains("└───────┴───────┘"));
    }

    #[test]
    fn does_not_treat_regular_pipes_as_table_without_separator() {
        let rendered = render_markdown_ansi("a | b\nnot a table");
        assert_eq!(rendered, "a | b\nnot a table");
    }
}
