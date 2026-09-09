//! Native-shell bootstrap. Browser credentials remain a separate launch mode.
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopConnection {
    pub app_server_origin: String,
    pub token: String,
    pub workspace: String,
}

pub fn connection() -> Result<Option<DesktopConnection>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let value = js_sys::Reflect::get(
            &js_sys::global(),
            &wasm_bindgen::JsValue::from_str("__PROTEUS_DESKTOP__"),
        )
        .map_err(|_| "Desktop bootstrap is unavailable".to_owned())?;
        if value.is_undefined() {
            return Ok(None);
        }
        let json = js_sys::JSON::stringify(&value)
            .map_err(|_| "Invalid desktop bootstrap".to_owned())?
            .as_string()
            .ok_or("Invalid desktop bootstrap")?;
        let connection: DesktopConnection = serde_json::from_str(&json)
            .map_err(|error| format!("Invalid desktop bootstrap: {error}"))?;
        super::normalize_local_origin(&connection.app_server_origin)?;
        if connection.token.is_empty() {
            return Err("Empty desktop credential".to_owned());
        }
        Ok(Some(connection))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(None)
    }
}

pub fn is_desktop() -> bool {
    connection().is_ok_and(|value| value.is_some())
}

pub fn inspector_route(architecture: bool) -> &'static str {
    match (is_desktop(), architecture) {
        (true, true) => "/inspector.html?view=architecture",
        (true, false) => "/inspector.html",
        (false, true) => "/architecture",
        (false, false) => "/configs",
    }
}
