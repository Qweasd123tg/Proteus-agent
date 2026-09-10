#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(raw_module = "/ui/layout.js")]
extern "C" {
    #[wasm_bindgen(js_name = preparePanelMotion)]
    pub(crate) fn prepare_panel_motion(selector: &str);
    #[wasm_bindgen(js_name = cancelLayoutMotion)]
    pub(crate) fn cancel_layout_motion();
    #[wasm_bindgen(js_name = mountComposerDock)]
    fn mount_composer_dock(root: &web_sys::Element) -> js_sys::Function;
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn prepare_panel_motion(_selector: &str) {}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn cancel_layout_motion() {}

#[cfg(target_arch = "wasm32")]
pub(crate) fn attach_composer(root: NodeRef<leptos::html::Form>) {
    Effect::new(move |_| {
        let Some(root) = root.get() else { return };
        let dispose = StoredValue::new_local(mount_composer_dock(root.as_ref()));
        on_cleanup(move || {
            dispose.with_value(|dispose| {
                let _ = dispose.call0(&JsValue::NULL);
            })
        });
    });
}
