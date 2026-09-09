mod backend;
mod preferences;
mod windows;

use anyhow::Result;
use preferences::Preferences;
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::{
    AppHandle, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
};
use tauri_plugin_dialog::DialogExt;

struct DesktopState {
    backend: Mutex<Option<backend::Backend>>,
    preferences_file: PathBuf,
    resources: PathBuf,
    error: Mutex<Option<String>>,
    stopping: AtomicBool,
}

#[derive(Serialize)]
struct LauncherState {
    preferences: Preferences,
    profiles: Vec<String>,
    auto_start: bool,
    running: bool,
    error: Option<String>,
}

#[tauri::command]
fn launcher_state(state: tauri::State<'_, DesktopState>) -> Result<LauncherState, String> {
    let saved = preferences::load(&state.preferences_file).map_err(display_error)?;
    let error = state.error.lock().unwrap().take();
    let running = state.backend.lock().unwrap().is_some();
    let explicit_workspace = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    let auto_start =
        (saved.is_some() || explicit_workspace.is_some()) && error.is_none() && !running;
    let mut preferences = saved.unwrap_or_else(|| Preferences {
        workspace: std::env::var("HOME").unwrap_or_default(),
        config: "codex".to_owned(),
    });
    if let Some(path) = explicit_workspace {
        preferences.workspace = path.display().to_string();
    }
    let profiles = preferences::profiles(&preferences::config_dir().map_err(display_error)?)
        .map_err(display_error)?;
    Ok(LauncherState {
        preferences,
        profiles,
        auto_start,
        running,
        error,
    })
}

#[tauri::command]
async fn choose_workspace(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Выберите папку проекта")
            .blocking_pick_folder()
            .map(|path| {
                path.into_path()
                    .map(|p| p.display().to_string())
                    .map_err(|e| e.to_string())
            })
            .transpose()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn start_agent(app: AppHandle, preferences: Preferences) -> Result<(), String> {
    let launch_app = app.clone();
    let connection = tauri::async_runtime::spawn_blocking(move || -> Result<_> {
        let state = launch_app.state::<DesktopState>();
        let mut backend = state.backend.lock().unwrap();
        // Submission in the project chooser explicitly ends the previous session.
        drop(backend.take());
        let workspace = PathBuf::from(&preferences.workspace).canonicalize()?;
        let launched = backend::Backend::launch(
            &state.resources.join("bin"),
            &workspace,
            &preferences.config,
            &state.stopping,
        )?;
        let connection = launched.connection.clone();
        preferences::save(&state.preferences_file, &preferences)?;
        *backend = Some(launched);
        Ok(connection)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(display_error)?;
    for label in ["main", "inspector"] {
        if let Some(window) = app.get_webview_window(label) {
            window.destroy().map_err(|e| e.to_string())?;
        }
    }
    windows::client(&app, "main", &connection).map_err(display_error)?;
    if let Some(window) = app.get_webview_window("launcher") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn open_client(app: AppHandle, label: String) -> Result<(), String> {
    let connection = app
        .state::<DesktopState>()
        .backend
        .lock()
        .unwrap()
        .as_ref()
        .map(|backend| backend.connection.clone())
        .ok_or("Backend не запущен")?;
    windows::client(&app, &label, &connection).map_err(display_error)
}

fn display_error(error: anyhow::Error) -> String {
    format!("{error:#}")
}

fn setup(app: &mut tauri::App) -> Result<()> {
    let resources = app.path().resource_dir()?;
    let install_error = preferences::config_dir()
        .and_then(|directory| preferences::install_configs(&resources.join("configs"), &directory))
        .err()
        .map(display_error);
    app.manage(DesktopState {
        backend: Mutex::new(None),
        preferences_file: app.path().app_config_dir()?.join("preferences.json"),
        resources,
        error: Mutex::new(install_error),
        stopping: AtomicBool::new(false),
    });
    let project = MenuItem::with_id(
        app,
        "project",
        "Открыть проект…",
        true,
        Some("CmdOrCtrl+Shift+O"),
    )?;
    let inspector = MenuItem::with_id(
        app,
        "inspector",
        "Inspector",
        true,
        Some("CmdOrCtrl+Shift+I"),
    )?;
    let quit = PredefinedMenuItem::quit(app, Some("Выйти из Proteus"))?;
    let submenu = Submenu::with_items(app, "Proteus", true, &[&project, &inspector, &quit])?;
    app.set_menu(Menu::with_items(app, &[&submenu])?)?;
    windows::launcher(app.handle())?;
    let handle = app.handle().clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let state = handle.state::<DesktopState>();
            let error = {
                let mut backend = state.backend.lock().unwrap();
                let error = backend.as_mut().and_then(|backend| backend.exit_error());
                if error.is_some() {
                    drop(backend.take());
                }
                error
            };
            if let Some(error) = error {
                *state.error.lock().unwrap() = Some(error);
                if let Some(window) = handle.get_webview_window("launcher") {
                    let _ = window.eval("location.reload()");
                }
                let _ = windows::launcher(&handle);
            }
        }
    });
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            let label = if app.get_webview_window("main").is_some() {
                "main"
            } else {
                "launcher"
            };
            let _ = windows::focus(app, label);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            launcher_state,
            choose_workspace,
            start_agent,
            open_client
        ])
        .setup(|app| setup(app).map_err(Into::into))
        .on_menu_event(|app, event| match event.id().as_ref() {
            "project" => {
                if let Some(window) = app.get_webview_window("launcher") {
                    let _ = window.eval("location.reload()");
                }
                let _ = windows::launcher(app);
            }
            "inspector" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = open_client(app, "inspector".to_owned()).await;
                });
            }
            _ => {}
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    "inspector" => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    "launcher"
                        if window
                            .state::<DesktopState>()
                            .backend
                            .try_lock()
                            .is_ok_and(|backend| backend.is_some()) =>
                    {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    _ => {
                        api.prevent_close();
                        window
                            .state::<DesktopState>()
                            .stopping
                            .store(true, Ordering::Relaxed);
                        window.app_handle().exit(0);
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Не удалось создать приложение Proteus")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                app.state::<DesktopState>()
                    .stopping
                    .store(true, Ordering::Relaxed);
                drop(app.state::<DesktopState>().backend.lock().unwrap().take());
            }
        });
}
