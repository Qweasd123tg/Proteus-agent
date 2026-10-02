use leptos::prelude::*;
use wasm_bindgen::{JsCast, prelude::*};

#[wasm_bindgen(raw_module = "/extensions/usage/details.js")]
extern "C" {
    #[wasm_bindgen(js_name = mountUsageDetails, catch)]
    fn mount_usage(
        root: &web_sys::Element,
        read_usage: &js_sys::Function,
    ) -> Result<js_sys::Function, JsValue>;
}

#[component]
pub(crate) fn UsageDetailsView(session_dir: ReadSignal<Option<String>>) -> impl IntoView {
    let root = NodeRef::<leptos::html::Div>::new();
    Effect::new(move |_| {
        let Some(element) = root.get() else { return };
        let Some(session) = session_dir.get() else {
            return;
        };
        let path = crate::api::session_path("/usage", &session);
        let reader =
            Closure::<dyn Fn(web_sys::AbortSignal) -> js_sys::Promise>::new(move |signal| {
                let path = path.clone();
                wasm_bindgen_futures::future_to_promise(async move {
                    crate::api::get_text_with_signal(&path, Some(&signal))
                        .await
                        .map(JsValue::from)
                        .map_err(|error| js_sys::Error::new(&error).into())
                })
            });
        let reader = StoredValue::new_local(reader);
        let mounted = reader
            .with_value(|reader| mount_usage(element.as_ref(), reader.as_ref().unchecked_ref()));
        match mounted {
            Ok(dispose) => {
                let dispose = StoredValue::new_local(dispose);
                on_cleanup(move || {
                    dispose.with_value(|callback| {
                        let _ = callback.call0(&JsValue::NULL);
                    })
                });
            }
            Err(_) => element.set_text_content(Some("Не удалось загрузить отчёт расхода")),
        }
    });
    view! { <section class="context-usage-report"><div node_ref=root class="usage-details-host"></div></section> }
}
