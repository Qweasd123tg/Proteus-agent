const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const ITALIC: &str = "\x1b[3m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";

pub(crate) fn render_markdown_ansi(text: &str) -> String {
    let mut rendered = String::new();
    let mut in_code_block = false;

    for raw_line in text.lines() {
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
            continue;
        }

        if in_code_block {
            rendered.push_str(DIM);
            rendered.push_str(line);
            rendered.push_str(RESET);
            rendered.push('\n');
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
            continue;
        }

        if let Some(quote) = trimmed.strip_prefix("> ") {
            rendered.push_str(DIM);
            rendered.push_str("│ ");
            rendered.push_str(RESET);
            rendered.push_str(&render_inline_ansi(quote));
            rendered.push('\n');
            continue;
        }

        if let Some((marker, body)) = list_item(trimmed) {
            rendered.push_str(YELLOW);
            rendered.push_str(marker);
            rendered.push_str(RESET);
            rendered.push(' ');
            rendered.push_str(&render_inline_ansi(body));
            rendered.push('\n');
            continue;
        }

        rendered.push_str(&render_inline_ansi(line));
        rendered.push('\n');
    }

    if text.ends_with('\n') {
        rendered
    } else {
        rendered.pop();
        rendered
    }
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
}
