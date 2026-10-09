//! Client package management; never installs or invokes agent process modules.
mod manifest;
mod store;
#[cfg(test)]
mod tests;

use base64::{Engine, engine::general_purpose::STANDARD};
use store::{InstalledPackage, PackageStore};
use tauri::{
    AppHandle, Manager, UriSchemeContext,
    http::{Request, Response},
};

fn store(app: &AppHandle) -> anyhow::Result<PackageStore> {
    Ok(PackageStore::new(
        app.path().app_local_data_dir()?.join("ui-extensions"),
    ))
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn install_ui_extension(
    app: AppHandle,
    archive: String,
    excluded_ids: Vec<String>,
) -> Result<InstalledPackage, String> {
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
        anyhow::ensure!(
            archive.len() <= store::MAX_ARCHIVE_BYTES * 4 / 3 + 4,
            "ZIP превышает 64 МиБ"
        );
        store(&app)?.install(&STANDARD.decode(archive)?, &excluded_ids)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) async fn remove_ui_extension(app: AppHandle, key: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || store(&app)?.remove(&key))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

pub(crate) fn serve(
    context: UriSchemeContext<'_, tauri::Wry>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let response = Response::builder()
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS")
        .header("Cache-Control", "no-store");
    if request.method() == "OPTIONS" {
        return response.status(204).body(Vec::new()).unwrap();
    }
    if request.method() != "GET" && request.method() != "HEAD" {
        return response.status(405).body(Vec::new()).unwrap();
    }
    let result = (|| -> anyhow::Result<_> {
        let (key, path) = request
            .uri()
            .path()
            .trim_start_matches('/')
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("Не указан ресурс пакета"))?;
        store(context.app_handle())?.read(key, path)
    })();
    match result {
        Ok((bytes, mime)) => response
            .header("Content-Type", mime)
            .body(if request.method() == "HEAD" {
                Vec::new()
            } else {
                bytes
            })
            .unwrap(),
        Err(error) => response
            .status(404)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(error.to_string().into_bytes())
            .unwrap(),
    }
}
