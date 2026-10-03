use serde_json::Value;
use web_sys::window;

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

pub(crate) fn short_id(id: impl ToString) -> String {
    let id = id.to_string();
    id.get(..8).unwrap_or(&id).to_owned()
}
pub(crate) fn format_token_count(tokens: u32) -> String {
    if tokens < 1000 {
        return tokens.to_string();
    }
    format!(
        "{}k",
        format!("{:.1}", f64::from(tokens) / 1000.0).trim_end_matches(".0")
    )
}
