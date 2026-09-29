use leptos::{html, prelude::*};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(raw_module = "/extensions/web-adapter.js")]
extern "C" {
    #[wasm_bindgen(js_name = mountClientWorkspace)]
    fn mount() -> js_sys::Function;
    #[wasm_bindgen(js_name = revealClientView)]
    fn reveal_view(view: &str);
}
pub(crate) fn reveal(view: &str) {
    #[cfg(target_arch = "wasm32")]
    reveal_view(view);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = view;
}
pub(crate) fn attach(root: NodeRef<html::Div>) {
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        if root.get().is_none() {
            return;
        }
        let dispose = StoredValue::new_local(mount());
        on_cleanup(move || {
            dispose.with_value(|f| {
                let _ = f.call0(&JsValue::NULL);
            })
        });
    });
    #[cfg(not(target_arch = "wasm32"))]
    let _ = root;
}
