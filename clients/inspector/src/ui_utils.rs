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
