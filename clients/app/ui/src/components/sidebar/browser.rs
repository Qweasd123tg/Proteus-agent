use super::preferences::Preferences;
use leptos::prelude::*;
use wasm_bindgen::{JsCast, prelude::*};

#[wasm_bindgen(raw_module = "/ui/sidebar.js")]
extern "C" {
    #[wasm_bindgen(js_name = mountSidebar)]
    fn mount_sidebar(root: &web_sys::Element, update: &js_sys::Function) -> js_sys::Function;
}
pub(super) fn attach(root: NodeRef<leptos::html::Aside>, preferences: Preferences) {
    Effect::new(move |_| {
        let Some(element) = root.get() else { return };
        let callback = Closure::<dyn Fn(String, String, String)>::new(move |action, id, value| {
            preferences.update(action, id, value)
        });
        let stop = mount_sidebar(element.as_ref(), callback.as_ref().unchecked_ref());
        let callback = StoredValue::new_local(callback);
        let stop = StoredValue::new_local(stop);
        on_cleanup(move || {
            stop.with_value(|stop| {
                let _ = stop.call0(&JsValue::NULL);
            });
            callback.dispose();
        });
    });
}
