use pulldown_cmark::{Event as MarkdownEvent, Options as MarkdownOptions, Parser, html};

pub(crate) fn markdown_html(text: &str) -> String {
    let mut options = MarkdownOptions::empty();
    options.insert(MarkdownOptions::ENABLE_TABLES);
    options.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    options.insert(MarkdownOptions::ENABLE_TASKLISTS);
    let normalized_text = normalize_math_code_blocks(text);
    let (markdown_text, math_fragments) = extract_math_fragments(&normalized_text);
    let parser = Parser::new_ext(&markdown_text, options).map(|event| match event {
        MarkdownEvent::Html(raw) | MarkdownEvent::InlineHtml(raw) => MarkdownEvent::Text(raw),
        event => event,
    });
    let mut output = String::new();
    html::push_html(&mut output, parser);
    for (token, html) in math_fragments {
        output = output.replace(&token, &html);
    }
    enhance_code_blocks(&output)
}

pub(crate) fn plain_text_html(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            '\n' => output.push_str("<br>"),
            _ => output.push(ch),
        }
    }
    output
}

/// Оборачивает каждый `<pre><code>` блок в контейнер с шапкой: ярлык языка,
/// кнопки copy и wrap (обработчик кликов делегирован в index.html). Поиск по
/// литералу безопасен: pulldown-cmark экранирует `<`/`>` внутри кода, поэтому
/// `</code></pre>` не встретится в содержимом блока.
fn enhance_code_blocks(html: &str) -> String {
    const OPEN: &str = "<pre><code";
    const CLOSE: &str = "</code></pre>";
    const PRE_LEN: usize = 5; // "<pre>"

    let mut out = String::with_capacity(html.len() + 96);
    let mut rest = html;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(gt) = after[PRE_LEN..].find('>').map(|index| PRE_LEN + index + 1) else {
            out.push_str(after);
            return out;
        };
        let Some(close) = after[gt..].find(CLOSE).map(|index| gt + index) else {
            out.push_str(after);
            return out;
        };
        let lang = code_block_language(&after[..gt]);
        let block = &after[..close + CLOSE.len()];
        out.push_str(&format!(
            "<div class=\"code-block\"><div class=\"code-block-head\">\
<span class=\"code-lang\">{lang}</span>\
<span class=\"code-actions\">\
<button class=\"code-wrap\" type=\"button\" title=\"Перенос строк\">wrap</button>\
<button class=\"code-copy\" type=\"button\" title=\"Скопировать код\">copy</button>\
</span></div>{block}</div>"
        ));
        rest = &after[close + CLOSE.len()..];
    }
    out.push_str(rest);
    out
}

fn code_block_language(open_tag: &str) -> String {
    if let Some(index) = open_tag.find("language-") {
        let lang = open_tag[index + "language-".len()..]
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '+' | '#'))
            .collect::<String>();
        if !lang.is_empty() {
            return lang;
        }
    }
    "code".to_owned()
}

fn normalize_math_code_blocks(text: &str) -> String {
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
                if candidate_trimmed.starts_with(fence) {
                    break;
                }
                block.push_str(candidate);
                end_index += 1;
            }

            if end_index < lines.len() && looks_like_math_block(&block) {
                output.push_str(&block);
                index = end_index + 1;
                continue;
            }
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

fn fence_marker(line: &str) -> Option<&'static str> {
    if line.starts_with("```") {
        Some("```")
    } else if line.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
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

fn extract_math_fragments(text: &str) -> (String, Vec<(String, String)>) {
    let mut output = String::with_capacity(text.len());
    let mut fragments = Vec::new();
    let mut index = 0;
    let mut at_line_start = true;
    let mut in_fence: Option<String> = None;

    while index < text.len() {
        let rest = &text[index..];

        if at_line_start {
            let trimmed = rest.trim_start_matches([' ', '\t']);
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                let marker = trimmed.chars().take(3).collect::<String>();
                if in_fence.as_deref() == Some(marker.as_str()) {
                    in_fence = None;
                } else if in_fence.is_none() {
                    in_fence = Some(marker);
                }
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

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Лёгкая подсветка превью tool-вызовов: JSON-аргументы, унифицированные diff'ы
/// и всё остальное (без ложной раскраски). Возвращает безопасный HTML —
/// весь текст экранируется, цвет навешивается классами `tk-*`.
pub(crate) fn highlight_preview(text: &str) -> String {
    let trimmed = text.trim_start();
    if looks_like_apply_patch(trimmed) {
        highlight_apply_patch(text)
    } else if trimmed.starts_with('{') || trimmed.starts_with('[') {
        highlight_json(text)
    } else if looks_like_diff(text) {
        highlight_diff(text)
    } else {
        escape_html(text)
    }
}

fn looks_like_apply_patch(text: &str) -> bool {
    text.starts_with("*** Begin Patch")
}

fn looks_like_diff(text: &str) -> bool {
    text.lines().any(|line| {
        line.starts_with("@@")
            || line.starts_with("diff --git")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
    })
}

fn highlight_apply_patch(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut inner = String::new();
        let row = highlight_apply_patch_line(line, &mut inner);
        push_diff_line(&mut out, row, &inner);
    }
    out
}

/// Подсвечивает одну строку патча в `out` и возвращает класс-модификатор для
/// фоновой заливки всей строки (добавление/удаление), либо `None`.
fn highlight_apply_patch_line(line: &str, out: &mut String) -> Option<&'static str> {
    for (prefix, class) in [
        ("*** Add File: ", "tk-patch-op"),
        ("*** Delete File: ", "tk-patch-op"),
        ("*** Update File: ", "tk-patch-op"),
        ("*** Move to: ", "tk-patch-op"),
    ] {
        if let Some(path) = line.strip_prefix(prefix) {
            push_span(out, class, prefix);
            push_span(out, "tk-patch-path", path);
            return None;
        }
    }

    let class = if line == "*** Begin Patch" || line == "*** End Patch" {
        Some("tk-patch-boundary")
    } else if line == "*** End of File" {
        Some("tk-patch-meta")
    } else if line.starts_with("@@") {
        Some("tk-hunk")
    } else if line.starts_with('+') {
        Some("tk-add")
    } else if line.starts_with('-') {
        Some("tk-del")
    } else if line.starts_with(' ') {
        Some("tk-patch-context")
    } else {
        None
    };

    match class {
        Some(class) => push_span(out, class, line),
        None => out.push_str(&escape_html(line)),
    }
    diff_row_class(class)
}

fn highlight_diff(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let class = if line.starts_with("@@") {
            Some("tk-hunk")
        } else if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("diff ")
            || line.starts_with("index ")
        {
            Some("tk-meta")
        } else if line.starts_with('+') {
            Some("tk-add")
        } else if line.starts_with('-') {
            Some("tk-del")
        } else {
            None
        };
        let mut inner = String::new();
        match class {
            Some(class) => push_span(&mut inner, class, line),
            None => inner.push_str(&escape_html(line)),
        }
        push_diff_line(&mut out, diff_row_class(class), &inner);
    }
    out
}

/// Заливку всей строки даём только добавлениям и удалениям — остальные классы
/// (контекст, заголовки ханков, метаданные) остаются без фона.
fn diff_row_class(class: Option<&'static str>) -> Option<&'static str> {
    match class {
        Some("tk-add") => Some("tk-row-add"),
        Some("tk-del") => Some("tk-row-del"),
        _ => None,
    }
}

/// Оборачивает строку диффа в блочный `tk-line`, чтобы фон добавления/удаления
/// тянулся на всю ширину. Строки идут встык без литерального `\n`.
fn push_diff_line(out: &mut String, row: Option<&'static str>, inner: &str) {
    out.push_str("<span class=\"tk-line");
    if let Some(row) = row {
        out.push(' ');
        out.push_str(row);
    }
    out.push_str("\">");
    out.push_str(inner);
    out.push_str("</span>");
}

fn push_span(out: &mut String, class: &str, text: &str) {
    out.push_str("<span class=\"");
    out.push_str(class);
    out.push_str("\">");
    out.push_str(&escape_html(text));
    out.push_str("</span>");
}

fn highlight_json(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                let mut raw = String::from('"');
                let mut escaped = false;
                for next in chars.by_ref() {
                    raw.push(next);
                    if escaped {
                        escaped = false;
                    } else if next == '\\' {
                        escaped = true;
                    } else if next == '"' {
                        break;
                    }
                }
                // Строка-ключ, если следующий значимый символ — двоеточие.
                let is_key = chars
                    .clone()
                    .find(|c| !c.is_whitespace())
                    .is_some_and(|c| c == ':');
                let class = if is_key { "tk-key" } else { "tk-str" };
                out.push_str("<span class=\"");
                out.push_str(class);
                out.push_str("\">");
                out.push_str(&escape_html(&raw));
                out.push_str("</span>");
            }
            '{' | '}' | '[' | ']' | ':' | ',' => {
                out.push_str("<span class=\"tk-punct\">");
                out.push(ch);
                out.push_str("</span>");
            }
            '-' | '0'..='9' => {
                let mut num = String::from(ch);
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() || matches!(next, '.' | 'e' | 'E' | '+' | '-') {
                        num.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push_str("<span class=\"tk-num\">");
                out.push_str(&num);
                out.push_str("</span>");
            }
            't' | 'f' | 'n' => {
                let mut word = String::from(ch);
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_alphabetic() {
                        word.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if matches!(word.as_str(), "true" | "false" | "null") {
                    out.push_str("<span class=\"tk-bool\">");
                    out.push_str(&word);
                    out.push_str("</span>");
                } else {
                    out.push_str(&escape_html(&word));
                }
            }
            _ => out.push_str(&escape_html(&ch.to_string())),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_html_preserves_inline_math_for_mathjax() {
        let html = markdown_html("Energy: $E = mc^2$.");

        assert!(html.contains(r#"<span class="mathjax-inline">\(E = mc^2\)</span>"#));
    }

    #[test]
    fn plain_text_html_escapes_markup_and_preserves_newlines() {
        let html = plain_text_html("<b>one</b>\n& two");

        assert_eq!(html, "&lt;b&gt;one&lt;/b&gt;<br>&amp; two");
    }

    #[test]
    fn highlight_preview_colors_json_keys_and_values() {
        let html = highlight_preview("{\n  \"path\": \"a.rs\",\n  \"n\": 12\n}");

        assert!(html.contains("<span class=\"tk-key\">\"path\"</span>"));
        assert!(html.contains("<span class=\"tk-str\">\"a.rs\"</span>"));
        assert!(html.contains("<span class=\"tk-num\">12</span>"));
    }

    #[test]
    fn highlight_preview_colors_diff_lines() {
        let html = highlight_preview("@@ -1 +1 @@\n-old\n+new");

        assert!(html.contains("<span class=\"tk-hunk\">@@ -1 +1 @@</span>"));
        assert!(html.contains("<span class=\"tk-del\">-old</span>"));
        assert!(html.contains("<span class=\"tk-add\">+new</span>"));
    }

    #[test]
    fn highlight_preview_colors_apply_patch_lines() {
        let html = highlight_preview(
            "*** Begin Patch\n*** Update File: src/lib.rs\n@@ old\n-old\n+new\n*** End Patch",
        );

        assert!(html.contains("<span class=\"tk-patch-boundary\">*** Begin Patch</span>"));
        assert!(html.contains("<span class=\"tk-patch-op\">*** Update File: </span>"));
        assert!(html.contains("<span class=\"tk-patch-path\">src/lib.rs</span>"));
        assert!(html.contains("<span class=\"tk-hunk\">@@ old</span>"));
        assert!(html.contains("<span class=\"tk-del\">-old</span>"));
        assert!(html.contains("<span class=\"tk-add\">+new</span>"));
    }

    #[test]
    fn highlight_preview_escapes_markup_in_all_modes() {
        // JSON-режим: значение с тегами не должно протечь как разметка.
        assert!(highlight_preview("{\"x\": \"<img>\"}").contains("&lt;img&gt;"));
        // Patch-режим: путь в заголовке тоже экранируется.
        assert!(highlight_preview("*** Begin Patch\n*** Add File: <bad>").contains("&lt;bad&gt;"));
        // Generic-режим экранирует целиком.
        assert_eq!(highlight_preview("<script>"), "&lt;script&gt;");
    }

    #[test]
    fn markdown_html_preserves_display_math_for_mathjax() {
        let html = markdown_html(r"\[\int_0^1 x^2 dx = \frac{1}{3}\]");

        assert!(
            html.contains(
                r#"<span class="mathjax-display">\[\int_0^1 x^2 dx = \frac{1}{3}\]</span>"#
            )
        );
    }

    #[test]
    fn markdown_html_does_not_extract_math_inside_code_spans() {
        let html = markdown_html("Use `$x$` literally.");

        assert!(html.contains("<code>$x$</code>"));
        assert!(!html.contains("mathjax-inline"));
    }

    #[test]
    fn markdown_html_renders_math_only_fenced_code_blocks() {
        let html = markdown_html("```tex\n$$a^2 + b^2 = c^2$$\n$$x = y$$\n```");

        assert!(html.contains(r#"<span class="mathjax-display">\[a^2 + b^2 = c^2\]</span>"#));
        assert!(html.contains(r#"<span class="mathjax-display">\[x = y\]</span>"#));
        assert!(!html.contains("<pre><code>"));
    }

    #[test]
    fn markdown_html_renders_math_only_indented_code_blocks() {
        let html = markdown_html("    $$a^2 + b^2 = c^2$$\n    $$x = y$$");

        assert!(html.contains(r#"<span class="mathjax-display">\[a^2 + b^2 = c^2\]</span>"#));
        assert!(html.contains(r#"<span class="mathjax-display">\[x = y\]</span>"#));
        assert!(!html.contains("<pre><code>"));
    }

    #[test]
    fn markdown_html_keeps_non_math_fenced_code_blocks_as_code() {
        let html = markdown_html("```rust\nlet price = \"$10\";\n```");

        assert!(html.contains("<pre><code"));
        assert!(html.contains("let price"));
        assert!(!html.contains("mathjax"));
    }

    #[test]
    fn markdown_html_wraps_code_blocks_with_language_label_and_actions() {
        let html = markdown_html("```rust\nfn main() {}\n```");

        assert!(html.contains("class=\"code-block\""));
        assert!(html.contains("<span class=\"code-lang\">rust</span>"));
        assert!(html.contains("class=\"code-copy\""));
        assert!(html.contains("class=\"code-wrap\""));
        assert!(html.contains("<pre><code"));
    }

    #[test]
    fn markdown_html_labels_unmarked_code_block_as_code() {
        let html = markdown_html("```\nplain text\n```");

        assert!(html.contains("<span class=\"code-lang\">code</span>"));
    }

    #[test]
    fn markdown_html_wraps_each_of_multiple_code_blocks() {
        let html = markdown_html("```py\na = 1\n```\n\ntext\n\n```js\nlet b = 2;\n```");

        assert_eq!(html.matches("class=\"code-block\"").count(), 2);
        assert!(html.contains("<span class=\"code-lang\">py</span>"));
        assert!(html.contains("<span class=\"code-lang\">js</span>"));
    }
}
