use std::cell::RefCell;

use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, Request, RequestInit, RequestMode, Response, window};

use crate::types::SessionToken;

const DEFAULT_APP_SERVER_ORIGIN: &str = "http://127.0.0.1:8787";
const SERVER_QUERY_KEYS: [&str; 4] = [
    "server",
    "app_server",
    "app_server_origin",
    "proteus_server",
];
const SESSION_QUERY_KEYS: [&str; 4] = ["token", "session", "session_token", "proteus_session"];
const SERVER_STORAGE_KEY: &str = "proteus.appServerOrigin";
const SESSION_STORAGE_KEY: &str = "proteus.sessionToken";
const SESSION_HEADER: &str = "X-Proteus-Session";

thread_local! {
    static APP_SERVER_ORIGIN: RefCell<String> = RefCell::new(DEFAULT_APP_SERVER_ORIGIN.to_owned());
    static SESSION_TOKEN: RefCell<SessionToken> = RefCell::new(SessionToken::missing());
}

pub(crate) fn load_session_token() -> Result<SessionToken, String> {
    load_app_server_origin()?;
    let token = if let Some(token) = query_session_token() {
        persist_session_token(&token)?;
        token
    } else if let Some(storage) = session_storage()? {
        if let Some(value) = storage.get_item(SESSION_STORAGE_KEY).map_err(js_error)? {
            SessionToken::new(value)
        } else {
            SessionToken::missing()
        }
    } else {
        SessionToken::missing()
    };

    SESSION_TOKEN.with(|stored| *stored.borrow_mut() = token.clone());
    Ok(token)
}

pub(crate) async fn get_json<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, String> {
    let text = get_text(path).await?;
    serde_json::from_str(&text).map_err(|error| format!("invalid response JSON: {error}"))
}

pub(crate) async fn post_json<T, R>(path: &str, body: &T) -> Result<R, String>
where
    T: Serialize,
    R: for<'de> Deserialize<'de>,
{
    let token = current_session_token();
    let request_body = serde_json::to_string(body).map_err(|error| error.to_string())?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_mode(RequestMode::Cors);
    init.set_body(&JsValue::from_str(&request_body));

    let headers = Headers::new().map_err(js_error)?;
    headers
        .set("content-type", "application/json")
        .map_err(js_error)?;
    set_session_header(&headers, &token)?;
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
    serde_json::from_str(&text).map_err(|error| format!("invalid response JSON: {error}"))
}

pub(crate) async fn get_text(path: &str) -> Result<String, String> {
    let token = current_session_token();
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_mode(RequestMode::Cors);
    let headers = Headers::new().map_err(js_error)?;
    set_session_header(&headers, &token)?;
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

fn set_session_header(headers: &Headers, token: &SessionToken) -> Result<(), String> {
    if let Some(token) = token.as_deref() {
        headers.set(SESSION_HEADER, token).map_err(js_error)?;
    }
    Ok(())
}

fn current_session_token() -> SessionToken {
    SESSION_TOKEN.with(|stored| stored.borrow().clone())
}

fn load_app_server_origin() -> Result<(), String> {
    let origin = if let Some(origin) = query_app_server_origin() {
        persist_app_server_origin(&origin)?;
        origin
    } else if let Some(storage) = session_storage()? {
        storage
            .get_item(SERVER_STORAGE_KEY)
            .map_err(js_error)?
            .map(normalize_app_server_origin)
            .unwrap_or_else(|| DEFAULT_APP_SERVER_ORIGIN.to_owned())
    } else {
        DEFAULT_APP_SERVER_ORIGIN.to_owned()
    };

    APP_SERVER_ORIGIN.with(|stored| *stored.borrow_mut() = origin);
    Ok(())
}

fn app_server_origin() -> String {
    APP_SERVER_ORIGIN.with(|stored| stored.borrow().clone())
}

fn app_server_url(path: &str) -> String {
    format!("{}{}", app_server_origin(), path)
}

fn query_app_server_origin() -> Option<String> {
    query_value(&SERVER_QUERY_KEYS).map(normalize_app_server_origin)
}

fn query_session_token() -> Option<SessionToken> {
    query_value(&SESSION_QUERY_KEYS).map(SessionToken::new)
}

fn query_value(keys: &[&str]) -> Option<String> {
    let search = window()?.location().search().ok()?;
    let search = search.strip_prefix('?').unwrap_or(&search);
    for pair in search.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if keys.iter().any(|candidate| candidate == &key) {
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

fn persist_session_token(token: &SessionToken) -> Result<(), String> {
    let Some(value) = token.as_deref() else {
        return Ok(());
    };
    if let Some(storage) = session_storage()? {
        storage
            .set_item(SESSION_STORAGE_KEY, value)
            .map_err(js_error)?;
    }
    Ok(())
}

fn session_storage() -> Result<Option<web_sys::Storage>, String> {
    window()
        .ok_or_else(|| "window is unavailable".to_owned())?
        .session_storage()
        .map_err(js_error)
}

fn decode_uri_component(value: &str) -> Option<String> {
    js_sys::decode_uri_component(value).ok()?.as_string()
}

fn normalize_app_server_origin(origin: String) -> String {
    let origin = origin.trim().trim_end_matches('/');
    if origin.is_empty() {
        DEFAULT_APP_SERVER_ORIGIN.to_owned()
    } else {
        origin.to_owned()
    }
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
