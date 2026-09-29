use leptos::prelude::*;

#[component]
pub(super) fn Shortcuts() -> impl IntoView {
    let root = NodeRef::<leptos::html::Div>::new();
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::prelude::*;
        #[wasm_bindgen(raw_module = "/ui/shortcuts/settings.js")]
        extern "C" {
            #[wasm_bindgen(js_name = mountShortcutSettings)]
            fn mount(root: &web_sys::Element) -> js_sys::Function;
        }
        Effect::new(move |_| {
            let Some(root) = root.get() else { return };
            let dispose = StoredValue::new_local(mount(root.as_ref()));
            on_cleanup(move || {
                dispose.with_value(|f| {
                    let _ = f.call0(&JsValue::NULL);
                });
            });
        });
    }
    view! { <div class="shortcut-settings" node_ref=root/> }
}
