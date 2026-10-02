use super::*;
use proteus_app_common::session_selection::select_startup_session;
use proteus_contracts::app_protocol::{
    AppBootstrap as BootstrapResponse, AppSessionSummary,
    http::{NewSessionRequest, ResumeSessionRequest},
};

pub(crate) async fn initialize_selected_session() -> Result<String, String> {
    let bootstrap_text = get_text("/bootstrap").await?;
    let bootstrap: BootstrapResponse = serde_json::from_str(&bootstrap_text)
        .map_err(|error| format!("invalid response JSON: {error}"))?;
    let catalog = get_json::<Vec<AppSessionSummary>>("/sessions").await?;
    let requested = select_startup_session(
        query_value(SELECTED_SESSION_QUERY_KEY).or(load_stored_selected_session()?),
        bootstrap.session_dir,
        &catalog,
    );
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

fn load_stored_selected_session() -> Result<Option<String>, String> {
    let Some(storage) = session_storage()? else {
        return Ok(None);
    };
    storage
        .get_item(&selected_session_storage_key(&app_server_origin()))
        .map_err(js_error)
}

fn persist_selected_session(session_dir: &str) -> Result<(), String> {
    if let Some(storage) = session_storage()? {
        storage
            .set_item(
                &selected_session_storage_key(&app_server_origin()),
                session_dir,
            )
            .map_err(js_error)?;
    }
    let window = window().ok_or_else(|| "window is unavailable".to_owned())?;
    let location = window.location();
    let search = location.search().map_err(js_error)?;
    let mut pairs: Vec<String> = search
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter(|pair| {
            pair.split_once('=').map_or(*pair, |(key, _)| key) != SELECTED_SESSION_QUERY_KEY
        })
        .map(ToOwned::to_owned)
        .collect();
    pairs.push(format!(
        "{SELECTED_SESSION_QUERY_KEY}={}",
        encode_uri_component(session_dir)
    ));
    let path = location.pathname().map_err(js_error)?;
    let hash = location.hash().map_err(js_error)?;
    window
        .history()
        .map_err(js_error)?
        .replace_state_with_url(
            &JsValue::NULL,
            "",
            Some(&format!("{path}?{}{hash}", pairs.join("&"))),
        )
        .map_err(js_error)
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
        let catalog: Vec<AppSessionSummary> = serde_json::from_value(serde_json::json!([{
            "session_dir": "/tmp/session-b",
            "session_id": "12345678-0000-0000-0000-000000000000",
            "workspace_path": "/tmp/workspace",
            "message_count": 0,
            "updated_at_ms": null,
            "preview": null,
            "activity": null
        }]))
        .unwrap();
        // A valid saved selection wins; a removed/unsupported one cannot
        // override the server's fresh bootstrap (which need not be persisted).
        assert_eq!(
            select_startup_session(
                Some("/tmp/session-b".into()),
                bootstrap.session_dir.clone(),
                &catalog
            ),
            Some("/tmp/session-b".into())
        );
        assert_eq!(
            select_startup_session(
                Some("/tmp/old-session".into()),
                bootstrap.session_dir,
                &catalog
            ),
            Some("/tmp/session-a".into())
        );
        assert_eq!(
            select_startup_session(Some("/tmp/old-session".into()), None, &catalog),
            None
        );
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
