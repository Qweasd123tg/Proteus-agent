mod post;
pub(crate) use post::{post_json, post_json_for_admission, post_text_with_signal};

use std::cell::RefCell;

use proteus_app_common::{
    CredentialStorageUpdate, SessionCredential, normalize_local_origin, resolve_credential,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, Request, RequestInit, RequestMode, Response, window};

use crate::types::{SessionToken, StdioOutput};

const DEFAULT_APP_SERVER_ORIGIN: &str = "http://127.0.0.1:8787";
const SERVER_QUERY_KEY: &str = "server";
const TOKEN_QUERY_KEY: &str = "token";
const SESSION_DIR_QUERY_KEY: &str = "session_dir";
const SERVER_STORAGE_KEY: &str = "proteus.appServerOrigin";
const SESSION_CREDENTIAL_STORAGE_KEY: &str = "proteus.sessionCredential";

thread_local! {
    static APP_SERVER_ORIGIN: RefCell<String> = RefCell::new(DEFAULT_APP_SERVER_ORIGIN.to_owned());
    static SESSION_TOKEN: RefCell<SessionToken> = RefCell::new(SessionToken::missing());
}

pub(crate) fn load_session_token() -> Result<SessionToken, String> {
    if let Some(connection) = proteus_app_common::desktop::connection()? {
        APP_SERVER_ORIGIN.with(|stored| *stored.borrow_mut() = connection.app_server_origin);
        let token = SessionToken::new(connection.token);
        SESSION_TOKEN.with(|stored| *stored.borrow_mut() = token.clone());
        return Ok(token);
    }
    load_app_server_origin()?;
    let stored = load_stored_credential()?;
    let resolved = resolve_credential(&app_server_origin(), query_value(TOKEN_QUERY_KEY), stored);
    update_stored_credential(&resolved.storage_update)?;
    let token = resolved
        .token
        .map(SessionToken::new)
        .unwrap_or_else(SessionToken::missing);

    SESSION_TOKEN.with(|stored| *stored.borrow_mut() = token.clone());
    Ok(token)
}

pub(crate) fn event_stream_url(session_dir: &str) -> String {
    let token = current_session_token();
    let origin = app_server_origin();
    let mut url = format!(
        "{origin}/events?{SESSION_DIR_QUERY_KEY}={}",
        encode_uri_component(session_dir)
    );
    if let Some(token) = token.as_deref() {
        url.push_str(&format!(
            "&{TOKEN_QUERY_KEY}={}",
            encode_uri_component(token)
        ));
    }
    url
}

pub(crate) fn image_url(session_dir: &str, id: &str) -> String {
    let mut url = format!(
        "{}/image?session_dir={}&path={}",
        app_server_origin(),
        encode_uri_component(session_dir),
        encode_uri_component(id)
    );
    if let Some(token) = current_session_token().as_deref() {
        url.push_str(&format!("&token={}", encode_uri_component(token)));
    }
    url
}

pub(crate) fn session_path(path: &str, session_dir: &str) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };
    format!(
        "{path}{separator}{SESSION_DIR_QUERY_KEY}={}",
        encode_uri_component(session_dir)
    )
}

pub(crate) fn requested_session_dir() -> Option<String> {
    query_value(SESSION_DIR_QUERY_KEY).filter(|value| !value.trim().is_empty())
}

pub(crate) fn load_selected_session_dir() -> Result<Option<String>, String> {
    let Some(storage) = session_storage()? else {
        return Ok(None);
    };
    storage
        .get_item(&selected_session_storage_key())
        .map_err(js_error)
}

pub(crate) fn persist_selected_session_dir(session_dir: &str) -> Result<(), String> {
    if let Some(storage) = session_storage()? {
        storage
            .set_item(&selected_session_storage_key(), session_dir)
            .map_err(js_error)?;
    }
    replace_requested_session_dir(Some(session_dir))
}

pub(crate) fn clear_selected_session_dir() -> Result<(), String> {
    if let Some(storage) = session_storage()? {
        storage
            .remove_item(&selected_session_storage_key())
            .map_err(js_error)?;
    }
    replace_requested_session_dir(None)
}

fn selected_session_storage_key() -> String {
    proteus_app_common::selected_session_storage_key(&app_server_origin())
}

fn replace_requested_session_dir(session_dir: Option<&str>) -> Result<(), String> {
    let window = window().ok_or_else(|| "window is unavailable".to_owned())?;
    let location = window.location();
    let search = location.search().map_err(js_error)?;
    let mut pairs: Vec<String> = search
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter(|pair| pair.split_once('=').map_or(*pair, |(key, _)| key) != SESSION_DIR_QUERY_KEY)
        .map(ToOwned::to_owned)
        .collect();
    if let Some(session_dir) = session_dir {
        pairs.push(format!(
            "{SESSION_DIR_QUERY_KEY}={}",
            encode_uri_component(session_dir)
        ));
    }
    let query = if pairs.is_empty() {
        String::new()
    } else {
        format!("?{}", pairs.join("&"))
    };
    let path = location.pathname().map_err(js_error)?;
    let hash = location.hash().map_err(js_error)?;
    window
        .history()
        .map_err(js_error)?
        .replace_state_with_url(&JsValue::NULL, "", Some(&format!("{path}{query}{hash}")))
        .map_err(js_error)
}

pub(crate) async fn get_json<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, String> {
    let text = get_text(path).await?;
    serde_json::from_str(&text).map_err(|error| format!("invalid response JSON: {error}"))
}

pub(crate) async fn get_text(path: &str) -> Result<String, String> {
    get_text_with_signal(path, None).await
}

pub(crate) async fn get_text_with_signal(
    path: &str,
    signal: Option<&web_sys::AbortSignal>,
) -> Result<String, String> {
    let token = current_session_token();
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_mode(RequestMode::Cors);
    init.set_signal(signal);
    let headers = Headers::new().map_err(js_error)?;
    set_authorization_header(&headers, &token)?;
    init.set_headers(headers.as_ref());
    let request = Request::new_with_str_and_init(&app_server_url(path), &init).map_err(js_error)?;
    let response_value = JsFuture::from(
        window()
            .ok_or_else(|| "window is unavailable".to_owned())?
            .fetch_with_request(&request),
    )
    .await
    .map_err(js_error)?;
    let response = response_value.dyn_into::<Response>().map_err(js_error)?;
    let status = response.status();
    let text_value = JsFuture::from(response.text().map_err(js_error)?)
        .await
        .map_err(js_error)?;
    let text = text_value
        .as_string()
        .ok_or_else(|| "response body is not text".to_owned())?;

    if !response.ok() {
        return Err(http_error(status, &text));
    }
    Ok(text)
}

fn set_authorization_header(headers: &Headers, token: &SessionToken) -> Result<(), String> {
    if let Some(token) = token.as_deref() {
        headers
            .set("authorization", &format!("Bearer {token}"))
            .map_err(js_error)?;
    }
    Ok(())
}

fn current_session_token() -> SessionToken {
    SESSION_TOKEN.with(|stored| stored.borrow().clone())
}

fn load_app_server_origin() -> Result<(), String> {
    let origin = if let Some(origin) = query_app_server_origin()? {
        persist_app_server_origin(&origin)?;
        origin
    } else if let Some(storage) = session_storage()? {
        match storage.get_item(SERVER_STORAGE_KEY).map_err(js_error)? {
            Some(origin) => normalize_local_origin(&origin)?,
            None => DEFAULT_APP_SERVER_ORIGIN.to_owned(),
        }
    } else {
        DEFAULT_APP_SERVER_ORIGIN.to_owned()
    };

    APP_SERVER_ORIGIN.with(|stored| *stored.borrow_mut() = origin);
    Ok(())
}

/// Internal diagnostics page; the native connection is inherited by its iframe.
pub(crate) fn diagnostics_url(session_dir: Option<&str>) -> String {
    let requested = requested_session_dir();
    let session_dir = session_dir.or(requested.as_deref());
    let mut url = session_dir
        .map(|dir| session_path("/inspector.html", dir))
        .unwrap_or_else(|| "/inspector.html".to_owned());
    if !proteus_app_common::desktop::is_desktop() {
        let separator = if url.contains('?') { '&' } else { '?' };
        url.push_str(&format!(
            "{separator}server={}",
            encode_uri_component(&app_server_origin())
        ));
        if let Some(token) = current_session_token().as_deref() {
            url.push_str(&format!("&token={}", encode_uri_component(token)));
        }
    }
    url
}

fn app_server_origin() -> String {
    APP_SERVER_ORIGIN.with(|stored| stored.borrow().clone())
}

fn app_server_url(path: &str) -> String {
    format!("{}{}", app_server_origin(), path)
}

fn query_app_server_origin() -> Result<Option<String>, String> {
    query_value(SERVER_QUERY_KEY)
        .map(|origin| normalize_local_origin(&origin))
        .transpose()
}

fn query_value(expected_key: &str) -> Option<String> {
    let search = window()?.location().search().ok()?;
    let search = search.strip_prefix('?').unwrap_or(&search);
    for pair in search.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == expected_key {
            let value = decode_uri_component(value).unwrap_or_else(|| value.to_owned());
            return Some(value);
        }
    }
    None
}

fn persist_app_server_origin(origin: &str) -> Result<(), String> {
    if let Some(storage) = session_storage()? {
        storage
            .set_item(SERVER_STORAGE_KEY, origin)
            .map_err(js_error)?;
    }
    Ok(())
}

fn load_stored_credential() -> Result<Option<SessionCredential>, String> {
    let Some(storage) = session_storage()? else {
        return Ok(None);
    };
    let Some(value) = storage
        .get_item(SESSION_CREDENTIAL_STORAGE_KEY)
        .map_err(js_error)?
    else {
        return Ok(None);
    };
    match serde_json::from_str(&value) {
        Ok(credential) => Ok(Some(credential)),
        Err(_) => {
            storage
                .remove_item(SESSION_CREDENTIAL_STORAGE_KEY)
                .map_err(js_error)?;
            Ok(None)
        }
    }
}

fn update_stored_credential(update: &CredentialStorageUpdate) -> Result<(), String> {
    let Some(storage) = session_storage()? else {
        return Ok(());
    };
    match update {
        CredentialStorageUpdate::Keep => Ok(()),
        CredentialStorageUpdate::Replace(credential) => storage
            .set_item(
                SESSION_CREDENTIAL_STORAGE_KEY,
                &serde_json::to_string(credential).map_err(|error| error.to_string())?,
            )
            .map_err(js_error),
        CredentialStorageUpdate::Remove => storage
            .remove_item(SESSION_CREDENTIAL_STORAGE_KEY)
            .map_err(js_error),
    }
}

fn session_storage() -> Result<Option<web_sys::Storage>, String> {
    window()
        .ok_or_else(|| "window is unavailable".to_owned())?
        .session_storage()
        .map_err(js_error)
}

fn encode_uri_component(value: &str) -> String {
    js_sys::encode_uri_component(value)
        .as_string()
        .unwrap_or_else(|| value.to_owned())
}

fn decode_uri_component(value: &str) -> Option<String> {
    js_sys::decode_uri_component(value).ok()?.as_string()
}

fn http_error(status: u16, text: &str) -> String {
    let kind = match status {
        400 => "malformed request",
        401 => "auth required or session expired",
        403 => "request denied",
        404 => "server endpoint not found",
        500..=599 => "server error",
        _ => "request failed",
    };
    format!("HTTP {status} ({kind}): {text}")
}

pub(crate) fn js_error(value: JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("JavaScript error: {value:?}"))
}
