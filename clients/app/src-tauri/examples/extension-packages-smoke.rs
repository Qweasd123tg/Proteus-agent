//! Explicit native WebKit/IPC gate. Run twice with the same isolated XDG dirs.
#[path = "../src/extension_packages/mod.rs"]
mod extension_packages;
#[path = "../src/graphics.rs"]
mod graphics;
#[path = "../src/local_transport.rs"]
mod local_transport;

use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    io::{Cursor, Write},
    path::PathBuf,
};
use tauri::{WebviewUrl, WebviewWindowBuilder, http::Response};

#[tauri::command]
fn smoke_fixture() -> String {
    let manifest = serde_json::json!({"apiVersion":4,"id":"archive-smoke","name":"Archive smoke","description":"Native fixture",
        "icon":{"src":"assets/icon.svg"},"preview":{"entry":"demo.js"},
        "views":[{"surfaces":["workspace"],"entry":"panel.js","requires":[],"layout":"scroll","isolation":"shadow"}]});
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in [
        ("extension.json", serde_json::to_vec(&manifest).unwrap()),
        ("panel.js", b"export {mount} from './lib/view.mjs'".to_vec()),
        ("demo.js", b"export {createServices} from './lib/demo.mjs'".to_vec()),
        ("lib/demo.mjs", b"export function createServices() {return {}}".to_vec()),
        ("lib/view.mjs", br#"export async function mount({root,signal}) {
          const css=document.createElement('link');css.rel='stylesheet';css.href=new URL('../assets/style.css',import.meta.url);root.append(css);
          const text=document.createElement('p');text.textContent=await (await fetch(new URL('../assets/text.txt',import.meta.url),{signal})).text();root.append(text);
          window.smokeMounted=(window.smokeMounted||0)+1;
          signal.addEventListener('abort',()=>window.smokeAborted=(window.smokeAborted||0)+1);
          return()=>window.smokeDisposed=(window.smokeDisposed||0)+1;
        }"#.to_vec()),
        ("assets/style.css", b"p {color: rgb(12, 34, 56)}".to_vec()),
        ("assets/text.txt", b"Relative package resource".to_vec()),
        ("assets/icon.svg", br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><circle cx="10" cy="10" r="8" fill="red"/></svg>"#.to_vec()),
    ] {
        zip.start_file(name, options).unwrap(); zip.write_all(&bytes).unwrap();
    }
    STANDARD.encode(zip.finish().unwrap().into_inner())
}

#[tauri::command]
fn smoke_report(app: tauri::AppHandle, error: Option<String>) {
    if let Some(error) = error {
        eprintln!("FAIL: {error}");
        app.exit(1);
    } else {
        println!("PASS: native ZIP install/import/assets/cold-load/remove");
        app.exit(0);
    }
}

fn main() {
    assert!(
        std::env::var_os("PROTEUS_EXTENSION_SMOKE_HOME").is_some(),
        "Use the isolated extension-packages-smoke.py runner"
    );
    unsafe {
        local_transport::configure_before_threads();
        graphics::configure_before_threads();
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui");
    tauri::Builder::default()
        .register_uri_scheme_protocol("proteus-extension", extension_packages::serve)
        .register_uri_scheme_protocol("proteus-smoke", move |_context, request| {
            let resource = request.uri().path().trim_start_matches('/');
            let (bytes, mime) = match resource {
                "" | "index.html" => (b"<!doctype html><meta charset='utf-8'><div id='settings'></div><div id='view'></div><script type='module' src='/smoke.js'></script>".to_vec(), "text/html".to_owned()),
                "smoke.js" => (include_bytes!("../tests/extension-packages-smoke.js").to_vec(), "text/javascript".to_owned()),
                _ => (std::fs::read(root.join(resource)).unwrap_or_default(), mime_guess::from_path(resource).first_or_octet_stream().to_string()),
            };
            Response::builder().header("Content-Type", mime).header("Access-Control-Allow-Origin", "*").body(bytes).unwrap()
        })
        .invoke_handler(tauri::generate_handler![extension_packages::install_ui_extension, extension_packages::remove_ui_extension, smoke_fixture, smoke_report])
        .setup(|app| {
            let phase = std::env::args().nth(1).unwrap_or_else(|| "install".into());
            WebviewWindowBuilder::new(app, "smoke", WebviewUrl::CustomProtocol(format!("proteus-smoke://localhost/index.html?phase={phase}").parse()?))
                .disable_drag_drop_handler().build()?;
            let handle = app.handle().clone();
            std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_secs(45)); eprintln!("FAIL: native ZIP gate timed out"); handle.exit(1); });
            Ok(())
        })
        .run(tauri::generate_context!()).expect("Native extension gate");
}
