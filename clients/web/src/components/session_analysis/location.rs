use wasm_bindgen::JsValue;
use web_sys::{Url, window};

const SESSION_KEY: &str = "analysis_session";
const VIEW_KEY: &str = "analysis_view";

fn current_url() -> Option<Url> {
    Url::new(&window()?.location().href().ok()?).ok()
}

pub(super) fn read() -> (Option<String>, bool) {
    let Some(url) = current_url() else {
        return (None, false);
    };
    let params = url.search_params();
    let session = params
        .get(SESSION_KEY)
        .filter(|value| !value.trim().is_empty());
    (session, params.get(VIEW_KEY).as_deref() == Some("context"))
}

/// Analysis selection is independent of the active chat's session_dir.
pub(super) fn persist(session: Option<&str>, context: bool) {
    let Some(url) = current_url() else { return };
    let params = url.search_params();
    match session {
        Some(session) => params.set(SESSION_KEY, session),
        None => params.delete(SESSION_KEY),
    }
    params.set(VIEW_KEY, if context { "context" } else { "requests" });
    if let Some(window) = window()
        && let Ok(history) = window.history()
    {
        let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&url.href()));
    }
}
