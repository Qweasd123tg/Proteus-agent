use leptos::{html, prelude::*};
use wasm_bindgen::prelude::*;
#[wasm_bindgen(raw_module = "/extensions/web-adapter.js")]
extern "C" {
    #[wasm_bindgen(js_name = mountClientSettings)]
    fn mount_settings(root: &web_sys::Element) -> js_sys::Function;
    #[wasm_bindgen(js_name = mountClientSlot)]
    fn mount_slot(root: &web_sys::Element, slot: &str) -> js_sys::Function;
}
pub(super) fn mount(root: NodeRef<html::Div>, slot: Option<&'static str>) {
    Effect::new(move |_| {
        let Some(root) = root.get() else { return };
        let dispose = StoredValue::new_local(match slot {
            Some(slot) => mount_slot(root.as_ref(), slot),
            None => mount_settings(root.as_ref()),
        });
        on_cleanup(move || {
            dispose.with_value(|f| {
                let _ = f.call0(&JsValue::NULL);
            })
        });
    });
}
#[component]
pub(super) fn ClientModuleSlot(surface: &'static str) -> impl IntoView {
    let root = NodeRef::<html::Div>::new();
    mount(root, Some(surface));
    view! { <div class="client-module-slot" data-client-slot=surface node_ref=root/> }
}
