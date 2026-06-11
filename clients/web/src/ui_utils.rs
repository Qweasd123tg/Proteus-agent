use serde_json::Value;
use web_sys::window;

pub(crate) fn compact_title(text: &str) -> String {
    let title = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Новая сессия")
        .trim();
    compact_text(title, 72)
}

pub(crate) fn first_words_title(text: &str, word_limit: usize) -> String {
    let title = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Новая сессия")
        .split_whitespace()
        .take(word_limit)
        .collect::<Vec<_>>()
        .join(" ");
    if title.trim().is_empty() {
        "Новая сессия".to_owned()
    } else {
        compact_text(&title, 72)
    }
}

pub(crate) fn compact_text(text: &str, limit: usize) -> String {
    if text.chars().count() > limit {
        format!("{}...", text.chars().take(limit).collect::<String>())
    } else {
        text.to_owned()
    }
}

pub(crate) fn output_text(output: &Value) -> String {
    output
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or("(empty response)")
        .to_owned()
}

pub(crate) fn compact_json(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "<invalid json>".to_owned());
    let limit = 180;
    if text.chars().count() > limit {
        format!("{}...", text.chars().take(limit).collect::<String>())
    } else {
        text
    }
}

pub(crate) fn copy_to_clipboard(text: String) {
    if let Some(window) = window() {
        let clipboard = window.navigator().clipboard();
        let _ = clipboard.write_text(&text);
    }
}

pub(crate) fn short_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_owned()
}

pub(crate) fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
