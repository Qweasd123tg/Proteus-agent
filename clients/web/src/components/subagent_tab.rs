//! The workspace owns the tab lifecycle; Leptos keeps owning its detail content.
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(raw_module = "/extensions/subagent-tabs.js")]
extern "C" {
    #[wasm_bindgen(js_name = openSubagentTab)]
    fn open_tab(
        key: &str,
        title: &str,
        root: &web_sys::Element,
        on_close: &js_sys::Function,
    ) -> bool;
    #[wasm_bindgen(js_name = closeSubagentTab)]
    fn close_tab(key: &str);
}

pub(super) fn open(
    key: &str,
    title: &str,
    root: &web_sys::HtmlElement,
    on_close: impl FnOnce() + 'static,
) {
    #[cfg(target_arch = "wasm32")]
    {
        let callback = Closure::once(on_close);
        if open_tab(key, title, root, callback.as_ref().unchecked_ref()) {
            callback.forget();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (key, title, root, on_close);
}

#[cfg(target_arch = "wasm32")]
pub(super) fn close(key: &str) {
    close_tab(key);
}
