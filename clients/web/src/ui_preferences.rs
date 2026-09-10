use crate::types::ContextUsage;
use web_sys::window;
const CONTEXT_USAGE_STORAGE_PREFIX: &str = "proteus.contextUsage:";

pub(crate) fn load_i32_setting(key: &str, fallback: i32) -> i32 {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key).ok().flatten())
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(fallback)
}

pub(crate) fn save_i32_setting(key: &str, value: i32) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, &value.to_string());
    }
}

pub(crate) fn load_bool_setting(key: &str, fallback: bool) -> bool {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key).ok().flatten())
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(fallback)
}

pub(crate) fn save_bool_setting(key: &str, value: bool) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, if value { "true" } else { "false" });
    }
}

/// Черновики композера: по ключу на сессию, переживают переключение и F5.
const DRAFT_STORAGE_PREFIX: &str = "proteus.draft:";

pub(crate) fn load_session_draft(session_dir: &str) -> Option<String> {
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| {
            storage
                .get_item(&format!("{DRAFT_STORAGE_PREFIX}{session_dir}"))
                .ok()
                .flatten()
        })
        .filter(|value| !value.trim().is_empty())
}

pub(crate) fn save_session_draft(session_dir: &str, draft: &str) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let key = format!("{DRAFT_STORAGE_PREFIX}{session_dir}");
        if draft.trim().is_empty() {
            let _ = storage.remove_item(&key);
        } else {
            let _ = storage.set_item(&key, draft);
        }
    }
}

pub(crate) fn remove_session_draft(session_dir: &str) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.remove_item(&format!("{DRAFT_STORAGE_PREFIX}{session_dir}"));
    }
}

/// Снимок контекста ключуется по сессии. Глобальный ключ здесь опасен:
/// после смены сессии — или модуля workflow/компактора, который перестал
/// слать TokenUsageUpdated, — бублик показывал бы хвост чужой сессии.
pub(crate) fn load_context_usage(session_dir: Option<&str>) -> Option<ContextUsage> {
    let session_dir = session_dir?;
    window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| {
            storage
                .get_item(&format!("{CONTEXT_USAGE_STORAGE_PREFIX}{session_dir}"))
                .ok()
                .flatten()
        })
        .and_then(|value| serde_json::from_str(&value).ok())
}

pub(crate) fn save_context_usage(session_dir: Option<&str>, usage: ContextUsage) {
    let Some(session_dir) = session_dir else {
        return;
    };
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten())
        && let Ok(value) = serde_json::to_string(&usage)
    {
        let _ = storage.set_item(
            &format!("{CONTEXT_USAGE_STORAGE_PREFIX}{session_dir}"),
            &value,
        );
    }
}

pub(crate) fn remove_context_usage(session_dir: &str) {
    if let Some(storage) = window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.remove_item(&format!("{CONTEXT_USAGE_STORAGE_PREFIX}{session_dir}"));
    }
}
