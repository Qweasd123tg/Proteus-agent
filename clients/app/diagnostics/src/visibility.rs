//! Retained diagnostics follow visibility of the owning workspace frame.
use leptos::{html, prelude::*};

pub(crate) fn watch(root: NodeRef<html::Section>) -> RwSignal<bool> {
    let visible = RwSignal::new(true);
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        use wasm_bindgen::{JsCast, closure::Closure, prelude::*};
        #[wasm_bindgen(raw_module = "/ui/modules/visibility.js")]
        extern "C" {
            #[wasm_bindgen(js_name = watchLogicalVisibility)]
            fn mount(root: &web_sys::Element, changed: &js_sys::Function) -> js_sys::Function;
        }
        let Some(root) = root.get() else { return };
        let callback =
            Closure::wrap(Box::new(move |shown: bool| visible.set(shown)) as Box<dyn FnMut(bool)>);
        let dispose = mount(root.as_ref(), callback.as_ref().unchecked_ref());
        let lifetime = StoredValue::new_local((dispose, callback));
        on_cleanup(move || {
            lifetime.with_value(|(dispose, _)| {
                let _ = dispose.call0(&JsValue::NULL);
            })
        });
    });
    #[cfg(not(target_arch = "wasm32"))]
    let _ = root;
    visible
}
