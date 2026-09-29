//! One shared dispatcher for web and native commands, with owner-bound cleanup.
#[cfg(target_arch = "wasm32")]
pub(crate) fn install(callback: impl FnMut(String) -> bool + 'static) {
    use leptos::prelude::*;
    use wasm_bindgen::{JsCast, prelude::*};
    #[wasm_bindgen(raw_module = "/ui/shortcuts/runtime.js")]
    extern "C" {
        #[wasm_bindgen(js_name = registerShortcuts)]
        fn register(handler: &js_sys::Function) -> js_sys::Function;
    }
    let callback = Closure::<dyn FnMut(String) -> bool>::new(callback);
    let dispose = StoredValue::new_local(register(callback.as_ref().unchecked_ref()));
    let callback = StoredValue::new_local(callback);
    on_cleanup(move || {
        dispose.with_value(|f| {
            let _ = f.call0(&JsValue::NULL);
        });
        callback.dispose();
    });
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install(_callback: impl FnMut(String) -> bool + 'static) {}
