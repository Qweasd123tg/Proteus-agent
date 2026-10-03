//! WebKitGTK hands paste events no clipboard images, so the Linux shell reads
//! them for the composer. Other webviews deliver images with the paste event.

/// PNG bytes of the clipboard image; empty when the clipboard holds none.
/// A sync command runs on the main thread, where GTK clipboard calls belong.
#[tauri::command]
pub(crate) fn read_clipboard_image() -> Result<tauri::ipc::Response, String> {
    clipboard_png().map(tauri::ipc::Response::new)
}

#[cfg(target_os = "linux")]
fn clipboard_png() -> Result<Vec<u8>, String> {
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    let Some(image) = clipboard.wait_for_image() else {
        return Ok(Vec::new());
    };
    image
        .save_to_bufferv("png", &[])
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "linux"))]
fn clipboard_png() -> Result<Vec<u8>, String> {
    Ok(Vec::new())
}
