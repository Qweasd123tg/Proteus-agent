use leptos::prelude::*;

#[component]
pub(crate) fn ExtensionsView() -> impl IntoView {
    let root = NodeRef::<leptos::html::Div>::new();
    #[cfg(target_arch = "wasm32")]
    browser::attach(root);
    view! { <div class="extension-host" node_ref=root aria-label="Панели расширений"></div> }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use wasm_bindgen::{JsCast, prelude::*};

    #[wasm_bindgen(raw_module = "/extensions/web-adapter.js")]
    extern "C" {
        #[wasm_bindgen(js_name = mountWebExtensions, catch)]
        fn mount_extensions(
            root: &web_sys::Element,
            read_config: &js_sys::Function,
            read_quota: &js_sys::Function,
        ) -> Result<js_sys::Function, JsValue>;
    }

    fn reader(path: &'static str) -> Closure<dyn Fn(web_sys::AbortSignal) -> js_sys::Promise> {
        Closure::<dyn Fn(web_sys::AbortSignal) -> js_sys::Promise>::new(move |signal| {
            wasm_bindgen_futures::future_to_promise(async move {
                crate::api::get_text_with_signal(path, Some(&signal))
                    .await
                    .map(JsValue::from)
                    .map_err(|error| js_sys::Error::new(&error).into())
            })
        })
    }

    pub(super) fn attach(root: NodeRef<leptos::html::Div>) {
        let readers = StoredValue::new_local((reader("/config"), reader("/model/quota")));
        Effect::new(move |_| {
            let Some(element) = root.get() else { return };
            let mounted = readers.with_value(|(config, quota)| {
                mount_extensions(
                    element.as_ref(),
                    config.as_ref().unchecked_ref(),
                    quota.as_ref().unchecked_ref(),
                )
            });
            match mounted {
                Ok(dispose) => {
                    let dispose = StoredValue::new_local(dispose);
                    on_cleanup(move || {
                        dispose.with_value(|callback| {
                            let _ = callback.call0(&JsValue::NULL);
                        });
                    });
                }
                Err(_) => element.set_text_content(Some("Не удалось загрузить панели расширений")),
            }
        });
    }
}
