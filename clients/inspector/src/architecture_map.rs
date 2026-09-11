use leptos::prelude::*;

#[component]
pub(crate) fn TopologyMapView(source: String) -> impl IntoView {
    let root = NodeRef::<leptos::html::Div>::new();
    #[cfg(target_arch = "wasm32")]
    browser::attach(root, source);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = source;
    view! { <div node_ref=root class="topology-explorer"></div> }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(raw_module = "/graph/view.js")]
    extern "C" {
        #[wasm_bindgen(js_name = mountTopologyGraph, catch)]
        fn mount(root: &web_sys::Element, source: &str) -> Result<js_sys::Function, JsValue>;
    }

    pub(super) fn attach(root: NodeRef<leptos::html::Div>, source: String) {
        Effect::new(move |_| {
            let Some(element) = root.get() else { return };
            match mount(element.as_ref(), &source) {
                Ok(dispose) => {
                    let dispose = StoredValue::new_local(dispose);
                    on_cleanup(move || {
                        dispose.with_value(|callback| {
                            let _ = callback.call0(&JsValue::NULL);
                        })
                    });
                }
                Err(_) => element.set_text_content(Some(
                    "Не удалось построить карту. Откройте каталог сборки или обновите данные.",
                )),
            }
        });
    }
}
