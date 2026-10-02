use super::escape_html;

/// Лёгкая подсветка превью tool-вызовов: JSON-аргументы, унифицированные diff'ы
/// и всё остальное (без ложной раскраски). Возвращает безопасный HTML —
/// весь текст экранируется, цвет навешивается классами `tk-*`.
pub(crate) fn highlight_preview(text: &str) -> String {
    let trimmed = text.trim_start();
    if looks_like_apply_patch(trimmed) {
        highlight_apply_patch(text)
    } else if trimmed.starts_with('{') || trimmed.starts_with('[') {
        highlight_json(text)
    } else if looks_like_git_status(text) {
        highlight_git_status(text)
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

/// Вывод `git status --short` (с `--branch` или без): первая строка может быть
/// заголовком ветки `## ...`, остальные непустые — записи `XY путь`. Требуем,
/// чтобы все непустые строки были валидными записями статуса — иначе обычный
/// текст с `## ` в начале (markdown-заголовок) ошибочно не попадёт сюда.
fn looks_like_git_status(text: &str) -> bool {
    let mut header = false;
    let mut entries = false;
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        if index == 0 && line.starts_with("## ") {
            header = true;
            continue;
        }
        if git_status_code_class(line).is_some() {
            entries = true;
        } else {
            return false;
        }
    }
    header || entries
}

fn highlight_git_status(text: &str) -> String {
    let mut out = String::new();
    for (index, line) in text.lines().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if line.starts_with("## ") {
            push_span(&mut out, "tk-git-branch", line);
        } else if let Some(class) = git_status_code_class(line) {
            let (code, rest) = line.split_at(2);
            push_span(&mut out, class, code);
            out.push_str(&escape_html(rest));
        } else {
            out.push_str(&escape_html(line));
        }
    }
    out
}

/// Класс для двухсимвольного XY-кода статуса (`??` untracked, `A` added,
/// `D` deleted, прочее — изменения), либо `None`, если строка не похожа на
/// запись `git status --short`.
fn git_status_code_class(line: &str) -> Option<&'static str> {
    let bytes = line.as_bytes();
    if bytes.len() < 3 || bytes[2] != b' ' {
        return None;
    }
    let valid = |byte: u8| {
        matches!(
            byte,
            b' ' | b'M' | b'A' | b'D' | b'R' | b'C' | b'U' | b'?' | b'!' | b'T'
        )
    };
    let (x, y) = (bytes[0], bytes[1]);
    if !valid(x) || !valid(y) || (x == b' ' && y == b' ') {
        return None;
    }
    let class = if x == b'?' || y == b'?' || x == b'!' || y == b'!' {
        "tk-git-untracked"
    } else if x == b'D' || y == b'D' {
        "tk-git-del"
    } else if x == b'A' || y == b'A' {
        "tk-git-add"
    } else {
        "tk-git-mod"
    };
    Some(class)
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
