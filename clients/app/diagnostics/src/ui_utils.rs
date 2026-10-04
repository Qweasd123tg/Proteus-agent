use serde_json::Value;
use wasm_bindgen::JsCast;
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

pub(crate) async fn copy_to_clipboard(text: String) -> Result<(), String> {
    let window = window().ok_or("clipboard is unavailable")?;
    let clipboard = js_sys::Reflect::get(window.navigator().as_ref(), &"clipboard".into())
        .map_err(|error| format!("{error:?}"))?
        .dyn_into::<web_sys::Clipboard>()
        .map_err(|_| "clipboard is unavailable".to_owned())?;
    wasm_bindgen_futures::JsFuture::from(clipboard.write_text(&text))
        .await
        .map_err(|error| format!("{error:?}"))?;
    Ok(())
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
