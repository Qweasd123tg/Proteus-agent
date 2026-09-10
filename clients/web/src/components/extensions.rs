use leptos::prelude::*;

#[component]
pub(crate) fn ExtensionsView(active_session_dir: ReadSignal<Option<String>>) -> impl IntoView {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = active_session_dir;
    let root = NodeRef::<leptos::html::Div>::new();
    #[cfg(target_arch = "wasm32")]
    browser::attach(root, active_session_dir, false);
    view! { <div class="extension-host" node_ref=root aria-label="Панели расширений"></div> }
}

#[component]
pub(crate) fn UsageDetailsView(session_dir: ReadSignal<Option<String>>) -> impl IntoView {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = session_dir;
    let root = NodeRef::<leptos::html::Div>::new();
    #[cfg(target_arch = "wasm32")]
    browser::attach(root, session_dir, true);
    view! {
        <section class="context-usage-report">
            <h2>"Запросы и расход"</h2>
            <p class="settings-hint">"Расход за всю историю чата, включая повторы и сжатие контекста. Размер текущего контекста показан отдельно выше."</p>
            <div node_ref=root class="usage-details-host"></div>
        </section>
    }
}

#[component]
pub(crate) fn ExtensionSettingsView() -> impl IntoView {
    let root = NodeRef::<leptos::html::Div>::new();
    #[cfg(target_arch = "wasm32")]
    browser::attach_settings(root);
    view! { <div class="extension-settings" node_ref=root></div> }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use wasm_bindgen::{JsCast, prelude::*};

    #[wasm_bindgen(raw_module = "/extensions/web-adapter.js")]
    extern "C" {
        #[wasm_bindgen(js_name = mountWebExtensionSettings, catch)]
        fn mount_settings(root: &web_sys::Element) -> Result<js_sys::Function, JsValue>;
        #[wasm_bindgen(js_name = mountWebExtensions, catch)]
        fn mount_extensions(
            root: &web_sys::Element,
            read_config: &js_sys::Function,
            read_quota: &js_sys::Function,
            read_usage: &js_sys::Function,
        ) -> Result<js_sys::Function, JsValue>;
        #[wasm_bindgen(js_name = mountUsageDetails, catch)]
        fn mount_usage(
            root: &web_sys::Element,
            read_usage: &js_sys::Function,
        ) -> Result<js_sys::Function, JsValue>;
    }

    pub(super) fn attach_settings(root: NodeRef<leptos::html::Div>) {
        Effect::new(move |_| {
            let Some(element) = root.get() else { return };
            match mount_settings(element.as_ref()) {
                Ok(dispose) => {
                    let dispose = StoredValue::new_local(dispose);
                    on_cleanup(move || {
                        dispose.with_value(|callback| {
                            let _ = callback.call0(&JsValue::NULL);
                        });
                    });
                }
                Err(_) => {
                    element.set_text_content(Some("Не удалось загрузить настройки расширений"))
                }
            }
        });
    }

    fn reader(path: String) -> Closure<dyn Fn(web_sys::AbortSignal) -> js_sys::Promise> {
        Closure::<dyn Fn(web_sys::AbortSignal) -> js_sys::Promise>::new(move |signal| {
            let path = path.clone();
            wasm_bindgen_futures::future_to_promise(async move {
                crate::api::get_text_with_signal(&path, Some(&signal))
                    .await
                    .map(JsValue::from)
                    .map_err(|error| js_sys::Error::new(&error).into())
            })
        })
    }

    pub(super) fn attach(
        root: NodeRef<leptos::html::Div>,
        session_dir: ReadSignal<Option<String>>,
        details: bool,
    ) {
        Effect::new(move |_| {
            let Some(element) = root.get() else { return };
            let path = session_dir
                .get()
                .map(|dir| {
                    format!(
                        "/usage?session_dir={}",
                        crate::api::encode_query_component(&dir)
                    )
                })
                .unwrap_or_else(|| "/usage".to_owned());
            let readers = StoredValue::new_local((
                reader("/config".into()),
                reader("/model/quota".into()),
                reader(path),
            ));
            let mounted = readers.with_value(|(config, quota, usage)| {
                if details {
                    return mount_usage(element.as_ref(), usage.as_ref().unchecked_ref());
                }
                mount_extensions(
                    element.as_ref(),
                    config.as_ref().unchecked_ref(),
                    quota.as_ref().unchecked_ref(),
                    usage.as_ref().unchecked_ref(),
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
