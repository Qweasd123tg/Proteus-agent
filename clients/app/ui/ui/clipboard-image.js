// WebKitGTK gives paste events no clipboard images, so the desktop shell
// reads them natively. Browsers deliver images with the event itself.
export async function readDesktopClipboardImage() {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return null;
  const buffer = await invoke("read_clipboard_image");
  return buffer?.byteLength
    ? new File([buffer], "image.png", { type: "image/png" })
    : null;
}
