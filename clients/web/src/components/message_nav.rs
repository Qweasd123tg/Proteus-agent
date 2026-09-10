use leptos::{html, prelude::*};

#[component]
pub(crate) fn MessageNav<J>(items: Memo<Vec<(u64, String)>>, on_jump: J) -> impl IntoView
where
    J: Fn(u64) + Copy + Send + 'static,
{
    let root = NodeRef::<html::Nav>::new();
    #[cfg(target_arch = "wasm32")]
    attach(root);
    view! {
        <nav class="msg-nav" node_ref=root aria-label="Переход к моим сообщениям"
            hidden=move || items.with(|items| items.len() < 2)
            class:dense=move || items.with(|items| items.len() > 60)
            style=move || format!("--message-count: {}", items.with(Vec::len))>
            <div class="msg-nav-track">
                <For each=move || items.get() key=|(id, _)| *id children=move |(id, text)| {
                    view! {
                        <button type="button" class="msg-nav-tick" tabindex="-1"
                            data-message-id=format!("msg-{id}") data-preview=text.clone()
                            aria-label=format!("К сообщению: {text}")
                            on:click=move |_| on_jump(id) />
                    }
                } />
            </div>
            <div class="msg-nav-preview" id="message-nav-preview" role="tooltip"><p></p></div>
        </nav>
    }
}

#[cfg(target_arch = "wasm32")]
fn attach(root: NodeRef<html::Nav>) {
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen(raw_module = "/ui/message-nav.js")]
    extern "C" {
        #[wasm_bindgen(js_name = mountMessageNav)]
        fn mount(root: &web_sys::Element) -> js_sys::Function;
    }
    Effect::new(move |_| {
        let Some(root) = root.get() else { return };
        let dispose = StoredValue::new_local(mount(root.as_ref()));
        on_cleanup(move || {
            dispose.with_value(|dispose| {
                let _ = dispose.call0(&JsValue::NULL);
            })
        });
    });
}
