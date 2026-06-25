use serde_json::Value;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::window;

pub(crate) fn set_timeout(duration_ms: i32, callback: impl FnOnce() + 'static) {
    if let Some(window) = window() {
        let closure = Closure::once_into_js(callback);
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            closure.unchecked_ref(),
            duration_ms,
        );
    }
}

pub(crate) fn compact_title(text: &str) -> String {
    let title = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Новая сессия")
        .trim();
    compact_text(title, 72)
}

pub(crate) fn compact_text(text: &str, limit: usize) -> String {
    if text.chars().count() > limit {
        format!("{}...", text.chars().take(limit).collect::<String>())
    } else {
        text.to_owned()
    }
}

pub(crate) fn relative_time_from_now(timestamp_ms: Option<u64>) -> String {
    let Some(timestamp_ms) = timestamp_ms else {
        return "давно".to_owned();
    };
    let now_ms = js_sys::Date::now().max(0.0) as u64;
    let elapsed_seconds = now_ms.saturating_sub(timestamp_ms) / 1000;

    if elapsed_seconds < 60 {
        "сейчас".to_owned()
    } else if elapsed_seconds < 60 * 60 {
        format!(
            "{} назад",
            ru_count(elapsed_seconds / 60, "минуту", "минуты", "минут")
        )
    } else if elapsed_seconds < 60 * 60 * 24 {
        format!(
            "{} назад",
            ru_count(elapsed_seconds / 60 / 60, "час", "часа", "часов")
        )
    } else if elapsed_seconds < 60 * 60 * 24 * 30 {
        format!(
            "{} назад",
            ru_count(elapsed_seconds / 60 / 60 / 24, "день", "дня", "дней")
        )
    } else if elapsed_seconds < 60 * 60 * 24 * 365 {
        format!(
            "{} назад",
            ru_count(
                elapsed_seconds / 60 / 60 / 24 / 30,
                "месяц",
                "месяца",
                "месяцев"
            )
        )
    } else {
        format!(
            "{} назад",
            ru_count(elapsed_seconds / 60 / 60 / 24 / 365, "год", "года", "лет")
        )
    }
}

fn ru_count(value: u64, one: &str, few: &str, many: &str) -> String {
    let rem_100 = value % 100;
    let rem_10 = value % 10;
    let word = if (11..=14).contains(&rem_100) {
        many
    } else {
        match rem_10 {
            1 => one,
            2..=4 => few,
            _ => many,
        }
    };
    format!("{value} {word}")
}

pub(crate) fn output_text(output: &Value) -> String {
    output
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or("(empty response)")
        .to_owned()
}

pub(crate) fn format_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "<invalid json>".to_owned())
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
