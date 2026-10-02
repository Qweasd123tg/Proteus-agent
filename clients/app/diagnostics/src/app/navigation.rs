use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{MouseEvent, Url, window};

#[derive(Clone, Copy)]
pub(super) struct Navigation {
    pub section: RwSignal<&'static str>,
}
impl Navigation {
    pub fn new() -> Self {
        let section = RwSignal::new(current());
        if let Some(window) = window() {
            let listener =
                Closure::<dyn FnMut(web_sys::Event)>::new(move |_| section.set(current()));
            let _ = window
                .add_event_listener_with_callback("popstate", listener.as_ref().unchecked_ref());
            let listener = StoredValue::new_local(listener);
            on_cleanup(move || {
                listener.with_value(|listener| {
                    let _ = window.remove_event_listener_with_callback(
                        "popstate",
                        listener.as_ref().unchecked_ref(),
                    );
                })
            });
        }
        Self { section }
    }
    pub fn href(self, section: &str) -> String {
        let Some(window) = window() else {
            return String::new();
        };
        let Ok(url) = Url::new(&window.location().href().unwrap_or_default()) else {
            return String::new();
        };
        // Keep connection and independent analysis selection across navigation.
        url.search_params().set("view", section);
        url.href()
    }
    pub fn click(self, event: MouseEvent, section: &'static str) {
        if event.button() != 0
            || event.ctrl_key()
            || event.meta_key()
            || event.shift_key()
            || event.alt_key()
        {
            return;
        }
        event.prevent_default();
        if self.section.get_untracked() == section {
            return;
        }
        if let Some(window) = window()
            && let Ok(history) = window.history()
        {
            let _ = history.push_state_with_url(&JsValue::NULL, "", Some(&self.href(section)));
        }
        self.section.set(section);
    }
}
fn current() -> &'static str {
    match crate::api::query_value("view").as_deref() {
        Some("analysis") => "analysis",
        Some("usage") => "usage",
        Some("architecture") => "architecture",
        Some("configs") => "configs",
        _ => match window()
            .and_then(|w| w.location().pathname().ok())
            .as_deref()
        {
            Some("/analysis") => "analysis",
            Some("/architecture") => "architecture",
            _ => "configs",
        },
    }
}
