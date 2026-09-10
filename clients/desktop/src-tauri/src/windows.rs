use anyhow::{Context, Result, bail};
use proteus_client_common::{desktop::DesktopConnection, selected_session_storage_key};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

pub fn focus(app: &AppHandle, label: &str) -> Result<()> {
    let window = app
        .get_webview_window(label)
        .context("Окно ещё не открыто")?;
    window.show()?;
    window.unminimize()?;
    window.set_focus()?;
    Ok(())
}

pub fn launcher(app: &AppHandle) -> Result<()> {
    if app.get_webview_window("launcher").is_some() {
        return focus(app, "launcher");
    }
    WebviewWindowBuilder::new(app, "launcher", WebviewUrl::App("launcher.html".into()))
        .title("Proteus — открыть проект")
        .inner_size(640.0, 560.0)
        .min_inner_size(520.0, 440.0)
        .build()?;
    Ok(())
}

pub fn client(
    app: &AppHandle,
    label: &str,
    connection: &DesktopConnection,
    session_dir: Option<&str>,
) -> Result<()> {
    if let Some(window) = app.get_webview_window(label) {
        if let Some(session_dir) = session_dir {
            let storage_key = selected_session_storage_key(&connection.app_server_origin);
            window.eval(format!(
                "(() => {{ sessionStorage.setItem({}, {}); const url = new URL(location.href); url.searchParams.set('session_dir', {}); location.href = url.href; }})();",
                serde_json::to_string(&storage_key)?,
                serde_json::to_string(session_dir)?,
                serde_json::to_string(session_dir)?,
            ))?;
        }
        return focus(app, label);
    }
    let (file, title) = match label {
        "main" => ("index.html", "Proteus"),
        "inspector" => ("inspector.html", "Proteus Inspector"),
        _ => bail!("Неизвестное окно"),
    };
    let bootstrap = format!(
        "Object.defineProperty(window, '__PROTEUS_DESKTOP__', {{value: Object.freeze({}), writable: false}});\n{}",
        serde_json::to_string(connection)?,
        include_str!("bridge.js")
    );
    let opener = app.clone();
    let new_window_opener = app.clone();
    let file = match session_dir {
        Some(session_dir) => format!("{file}?session_dir={}", percent_encode_query(session_dir)),
        None => file.to_owned(),
    };
    WebviewWindowBuilder::new(app, label, WebviewUrl::App(file.into()))
        .title(format!("{title} — {}", connection.workspace))
        .inner_size(1440.0, 940.0)
        .min_inner_size(860.0, 600.0)
        .initialization_script(bootstrap)
        .on_new_window(move |url, _| {
            if matches!(url.scheme(), "http" | "https" | "mailto") {
                let _ = new_window_opener
                    .opener()
                    .open_url(url.as_str(), None::<&str>);
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .on_navigation(move |url| {
            if matches!(url.scheme(), "tauri") && url.host_str() == Some("localhost") {
                return true;
            }
            if url.origin().ascii_serialization() == "http://tauri.localhost" {
                return true;
            }
            if cfg!(debug_assertions)
                && url.origin().ascii_serialization() == "http://127.0.0.1:1430"
            {
                return true;
            }
            if matches!(url.scheme(), "http" | "https" | "mailto") {
                let _ = opener.opener().open_url(url.as_str(), None::<&str>);
            }
            false
        })
        .build()?;
    Ok(())
}

fn percent_encode_query(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}
