use std::cell::RefCell;

use proteus_client_common::{
    CredentialStorageUpdate, SessionCredential, credential_for_client_link, normalize_local_origin,
    resolve_credential, selected_session_storage_key,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, Request, RequestInit, RequestMode, Response, window};

use crate::types::SessionToken;

const DEFAULT_APP_SERVER_ORIGIN: &str = "http://127.0.0.1:8787";
const DEFAULT_CHAT_ORIGIN: &str = "http://127.0.0.1:1420";
const SERVER_QUERY_KEY: &str = "server";
const CHAT_QUERY_KEY: &str = "chat";
const SESSION_QUERY_KEY: &str = "token";
const SERVER_STORAGE_KEY: &str = "proteus.appServerOrigin";
const CHAT_STORAGE_KEY: &str = "proteus.chatOrigin";
const SESSION_CREDENTIAL_STORAGE_KEY: &str = "proteus.sessionCredential";
const SELECTED_SESSION_QUERY_KEY: &str = "session_dir";

thread_local! {
    static APP_SERVER_ORIGIN: RefCell<String> = RefCell::new(DEFAULT_APP_SERVER_ORIGIN.to_owned());
    static CHAT_ORIGIN: RefCell<String> = RefCell::new(DEFAULT_CHAT_ORIGIN.to_owned());
    static SESSION_TOKEN: RefCell<SessionToken> = RefCell::new(SessionToken::missing());
    static SELECTED_SESSION_DIR: RefCell<Option<String>> = const { RefCell::new(None) };
}

use proteus_client_common::response::command_output;
use proteus_contracts::app_protocol::{
    AppBootstrap as BootstrapResponse, StdioOutput,
    http::{NewSessionRequest, ResumeSessionRequest},
};

pub(crate) fn load_session_token() -> Result<SessionToken, String> {
    if let Some(connection) = proteus_client_common::desktop::connection()? {
        APP_SERVER_ORIGIN.with(|stored| *stored.borrow_mut() = connection.app_server_origin);
        let token = SessionToken::new(connection.token);
        SESSION_TOKEN.with(|stored| *stored.borrow_mut() = token.clone());
        return Ok(token);
    }
    load_app_server_origin()?;
    load_chat_origin()?;
    let stored = load_stored_credential()?;
    let resolved = resolve_credential(&app_server_origin(), query_value(SESSION_QUERY_KEY), stored);
    update_stored_credential(&resolved.storage_update)?;
    let token = resolved
        .token
        .map(SessionToken::new)
        .unwrap_or_else(SessionToken::missing);

    SESSION_TOKEN.with(|stored| *stored.borrow_mut() = token.clone());
    Ok(token)
}

pub(crate) async fn get_json<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, String> {
    let text = get_text(path).await?;
    serde_json::from_str(&text).map_err(|error| format!("invalid response JSON: {error}"))
}

pub(crate) async fn initialize_selected_session() -> Result<String, String> {
    let bootstrap_text = get_text("/bootstrap").await?;
    let bootstrap: BootstrapResponse = serde_json::from_str(&bootstrap_text)
        .map_err(|error| format!("invalid response JSON: {error}"))?;
    let _workspace = &bootstrap.cwd;
    let requested = query_value(SELECTED_SESSION_QUERY_KEY)
        .or(load_stored_selected_session()?)
        .or(bootstrap
            .session_dir
            .map(|path| path.to_string_lossy().into_owned()));
    let id = format!("inspector-{}", js_sys::Date::now() as u64);
    let summary: crate::types::ConfigSummary = match requested {
        Some(session_dir) => {
            let response = post_json::<_, StdioOutput>(
                "/resume",
                &ResumeSessionRequest {
                    id: Some(id),
                    session_dir: session_dir.into(),
                },
            )
            .await?;
            command_output(response)?
        }
        None => {
            let response = post_json::<_, StdioOutput>(
                "/new-session",
                &NewSessionRequest {
                    id: Some(id),
                    source_session_dir: None,
                },
            )
            .await?;
            command_output(response)?
        }
    };
    let session_dir = summary
        .session_dir
        .ok_or_else(|| "server did not return the selected session_dir".to_owned())?;
    persist_selected_session(&session_dir)?;
    SELECTED_SESSION_DIR.with(|stored| *stored.borrow_mut() = Some(session_dir.clone()));
    Ok(session_dir)
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
    set_authorization_header(&headers, &token)?;
    init.set_headers(headers.as_ref());

    let path = selected_session_path(path);
    let request =
        Request::new_with_str_and_init(&app_server_url(&path), &init).map_err(js_error)?;
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
    set_authorization_header(&headers, &token)?;
    init.set_headers(headers.as_ref());
    let path = selected_session_path(path);
    let request =
        Request::new_with_str_and_init(&app_server_url(&path), &init).map_err(js_error)?;
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

fn load_chat_origin() -> Result<(), String> {
    let origin = if let Some(origin) = query_value(CHAT_QUERY_KEY) {
        let origin = normalize_local_origin(&origin)?;
        if let Some(storage) = session_storage()? {
            storage
                .set_item(CHAT_STORAGE_KEY, &origin)
                .map_err(js_error)?;
        }
        origin
    } else if let Some(storage) = session_storage()? {
        storage
            .get_item(CHAT_STORAGE_KEY)
            .map_err(js_error)?
            .map(|origin| normalize_local_origin(&origin))
            .transpose()?
            .unwrap_or_else(|| DEFAULT_CHAT_ORIGIN.to_owned())
    } else {
        DEFAULT_CHAT_ORIGIN.to_owned()
    };

    CHAT_ORIGIN.with(|stored| *stored.borrow_mut() = origin);
    Ok(())
}

/// Ссылка на chat-клиент с пробросом session token и app-server origin —
/// зеркально `inspector_link_url()` в `clients/web`: hardcoded href терял бы
/// token при включённом token-режиме и нестандартных портах.
pub(crate) fn chat_link_url() -> String {
    if proteus_client_common::desktop::is_desktop() {
        return SELECTED_SESSION_DIR.with(|stored| match stored.borrow().as_deref() {
            Some(session_dir) => format!(
                "proteus-desktop:chat?session_dir={}",
                encode_uri_component(session_dir)
            ),
            None => "proteus-desktop:chat".to_owned(),
        });
    }
    let origin = CHAT_ORIGIN.with(|stored| stored.borrow().clone());
    let mut params = Vec::new();
    let current_token = current_session_token();
    if let Some(token) =
        credential_for_client_link(&origin, DEFAULT_CHAT_ORIGIN, current_token.as_deref())
    {
        params.push(format!("token={}", encode_uri_component(token)));
    }
    params.push(format!(
        "server={}",
        encode_uri_component(&app_server_origin())
    ));
    SELECTED_SESSION_DIR.with(|stored| {
        if let Some(session_dir) = stored.borrow().as_deref() {
            params.push(format!("session_dir={}", encode_uri_component(session_dir)));
        }
    });
    format!("{origin}/?{}", params.join("&"))
}

pub(crate) fn app_server_origin() -> String {
    APP_SERVER_ORIGIN.with(|stored| stored.borrow().clone())
}

pub(crate) fn has_session_token() -> bool {
    SESSION_TOKEN.with(|stored| stored.borrow().as_deref().is_some())
}

fn app_server_url(path: &str) -> String {
    format!("{}{}", app_server_origin(), path)
}

fn query_app_server_origin() -> Result<Option<String>, String> {
    query_value(SERVER_QUERY_KEY)
        .map(|origin| normalize_local_origin(&origin))
        .transpose()
}

pub(crate) fn query_value(expected_key: &str) -> Option<String> {
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

fn load_stored_selected_session() -> Result<Option<String>, String> {
    let Some(storage) = session_storage()? else {
        return Ok(None);
    };
    storage
        .get_item(&selected_session_storage_key(&app_server_origin()))
        .map_err(js_error)
}

fn persist_selected_session(session_dir: &str) -> Result<(), String> {
    let Some(storage) = session_storage()? else {
        return Ok(());
    };
    storage
        .set_item(
            &selected_session_storage_key(&app_server_origin()),
            session_dir,
        )
        .map_err(js_error)
}

fn selected_session_path(path: &str) -> String {
    // Bootstrap/session lifecycle endpoints establish the target and are global.
    if matches!(path, "/bootstrap" | "/resume" | "/new-session") {
        return path.to_owned();
    }
    SELECTED_SESSION_DIR.with(|stored| match stored.borrow().as_deref() {
        Some(session_dir) => {
            let separator = if path.contains('?') { '&' } else { '?' };
            format!(
                "{path}{separator}session_dir={}",
                encode_uri_component(session_dir)
            )
        }
        None => path.to_owned(),
    })
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

fn decode_uri_component(value: &str) -> Option<String> {
    js_sys::decode_uri_component(value).ok()?.as_string()
}

fn encode_uri_component(value: &str) -> String {
    js_sys::encode_uri_component(value).into()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_lifecycle_response_unwraps_config_payload() {
        let bootstrap: BootstrapResponse = serde_json::from_value(serde_json::json!({
            "session_dir": "/tmp/session-a",
            "cwd": "/tmp/workspace"
        }))
        .unwrap();
        assert_eq!(
            bootstrap.session_dir.as_deref(),
            Some(std::path::Path::new("/tmp/session-a"))
        );
        assert_eq!(bootstrap.cwd, std::path::Path::new("/tmp/workspace"));
        let response: StdioOutput = serde_json::from_value(serde_json::json!({
            "type": "response",
            "id": "inspector-1",
            "ok": true,
            "output": { "session_dir": "/tmp/session-a" },
            "error": null
        }))
        .unwrap();
        assert_eq!(
            command_output::<serde_json::Value>(response).unwrap()["session_dir"],
            "/tmp/session-a"
        );
    }

    #[test]
    fn session_lifecycle_response_preserves_protocol_error() {
        let response: StdioOutput = serde_json::from_value(serde_json::json!({
            "type": "response",
            "id": "inspector-2",
            "ok": false,
            "output": null,
            "error": "session is unavailable"
        }))
        .unwrap();
        assert_eq!(
            command_output::<serde_json::Value>(response).unwrap_err(),
            "session is unavailable"
        );
    }
}
