use super::escape_html;

pub(super) fn normalize_math_code_blocks(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start_matches([' ', '\t']);

        if let Some(fence) = fence_marker(trimmed) {
            let mut block = String::new();
            let mut end_index = index + 1;
            while end_index < lines.len() {
                let candidate = lines[end_index];
                let candidate_trimmed = candidate.trim_start_matches([' ', '\t']);
                if closes_fence(candidate_trimmed, fence) {
                    break;
                }
                block.push_str(candidate);
                end_index += 1;
            }

            if end_index < lines.len() && looks_like_math_block(&block) {
                output.push_str(&block);
            } else {
                // Keep an ordinary code block together, including its closing
                // fence; never reinterpret that closer as another opening.
                for original in &lines[index..(end_index + 1).min(lines.len())] {
                    output.push_str(original);
                }
            }
            index = (end_index + 1).min(lines.len());
            continue;
        }

        if is_indented_code_line(line) {
            let start = index;
            let mut block = String::new();
            while index < lines.len()
                && (is_indented_code_line(lines[index]) || lines[index].trim().is_empty())
            {
                block.push_str(lines[index]);
                index += 1;
            }

            let dedented = dedent_code_block(&block);
            if looks_like_math_block(&dedented) {
                output.push_str(&dedented);
            } else {
                for original in &lines[start..index] {
                    output.push_str(original);
                }
            }
            continue;
        }

        output.push_str(line);
        index += 1;
    }

    output
}

fn fence_marker(line: &str) -> Option<&str> {
    let marker = line.chars().next().filter(|ch| matches!(ch, '`' | '~'))?;
    let len = line.chars().take_while(|ch| *ch == marker).count();
    (len >= 3).then_some(&line[..len])
}

fn closes_fence(line: &str, fence: &str) -> bool {
    fence_marker(line)
        .is_some_and(|marker| marker.starts_with(fence) && line[marker.len()..].trim().is_empty())
}

fn is_indented_code_line(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
}

fn dedent_code_block(block: &str) -> String {
    block
        .split_inclusive('\n')
        .map(|line| {
            if let Some(stripped) = line.strip_prefix("    ") {
                stripped
            } else if let Some(stripped) = line.strip_prefix('\t') {
                stripped
            } else {
                line
            }
        })
        .collect()
}

fn looks_like_math_block(block: &str) -> bool {
    let non_empty = block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    !non_empty.is_empty() && non_empty.iter().all(|line| is_math_line(line))
}

fn is_math_line(line: &str) -> bool {
    (line.starts_with("$$") && line.ends_with("$$") && line.len() > 4)
        || (line.starts_with("\\[") && line.ends_with("\\]") && line.len() > 4)
}

pub(super) fn extract_math_fragments(text: &str) -> (String, Vec<(String, String)>) {
    let mut output = String::with_capacity(text.len());
    let mut fragments = Vec::new();
    let mut index = 0;
    let mut at_line_start = true;
    let mut in_fence: Option<String> = None;

    while index < text.len() {
        let rest = &text[index..];

        if at_line_start {
            let end = rest.find('\n').map_or(rest.len(), |pos| pos + 1);
            let line = &rest[..end];
            let trimmed = line.trim_start_matches([' ', '\t']);
            let marker = trimmed.chars().next().filter(|ch| matches!(ch, '`' | '~'));
            let marker_len =
                marker.map_or(0, |ch| trimmed.chars().take_while(|c| *c == ch).count());
            let was_in_fence = in_fence.is_some();
            if let Some(fence) = &in_fence {
                if trimmed.starts_with(fence)
                    && marker_len >= fence.len()
                    && trimmed[marker_len..].trim().is_empty()
                {
                    in_fence = None;
                }
            } else if marker_len >= 3 && line.len() - trimmed.len() <= 3 {
                in_fence = Some(trimmed[..marker_len].to_owned());
            }
            // Consume the closing fence as a line too. Otherwise its backticks
            // are mistaken for inline code spanning the following prose/math.
            if was_in_fence || in_fence.is_some() || is_indented_code_line(line) {
                output.push_str(line);
                at_line_start = line.ends_with('\n');
                index += end;
                continue;
            }
        }

        if let Some(ch) = rest.chars().next() {
            if in_fence.is_none() && ch == '`' {
                let tick_count = rest.chars().take_while(|next| *next == '`').count();
                let ticks = "`".repeat(tick_count);
                if let Some(end) = rest[tick_count..].find(&ticks) {
                    let end_index = tick_count + end + tick_count;
                    let segment = &rest[..end_index];
                    output.push_str(segment);
                    at_line_start = segment.ends_with('\n');
                    index += end_index;
                    continue;
                }
            }

            if in_fence.is_none()
                && let Some((delimiter, end_delimiter, display)) = math_start(rest)
            {
                let content_start = delimiter.len();
                if let Some(relative_end) = find_math_end(&rest[content_start..], end_delimiter) {
                    let content = &rest[content_start..content_start + relative_end];
                    let consumed = content_start + relative_end + end_delimiter.len();
                    let token = format!("PROTEUSMATH{}", fragments.len());
                    output.push_str(&token);
                    fragments.push((token, math_html(content, display)));
                    at_line_start = rest[..consumed].ends_with('\n');
                    index += consumed;
                    continue;
                }
            }

            output.push(ch);
            at_line_start = ch == '\n';
            index += ch.len_utf8();
        } else {
            break;
        }
    }

    (output, fragments)
}

fn math_start(text: &str) -> Option<(&'static str, &'static str, bool)> {
    if text.starts_with("\\[") {
        Some(("\\[", "\\]", true))
    } else if text.starts_with("\\(") {
        Some(("\\(", "\\)", false))
    } else if text.starts_with("$$") {
        Some(("$$", "$$", true))
    } else if text.starts_with('$') && !text.starts_with("$$") {
        Some(("$", "$", false))
    } else {
        None
    }
}

fn find_math_end(text: &str, delimiter: &str) -> Option<usize> {
    if delimiter == "$" {
        let mut escaped = false;
        for (index, ch) in text.char_indices() {
            if ch == '\\' {
                escaped = !escaped;
                continue;
            }
            if ch == '$' && !escaped {
                return Some(index);
            }
            escaped = false;
        }
        None
    } else {
        text.find(delimiter)
    }
}

fn math_html(content: &str, display: bool) -> String {
    let content = escape_html(content.trim());
    if display {
        format!(r#"<span class="mathjax-display">\[{content}\]</span>"#)
    } else {
        format!(r#"<span class="mathjax-inline">\({content}\)</span>"#)
    }
}
