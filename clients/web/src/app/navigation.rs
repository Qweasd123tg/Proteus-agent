use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{MouseEvent, window};

#[derive(Clone, Copy)]
pub(super) struct AppRouter {
    pub route: ReadSignal<String>,
    set_route: WriteSignal<String>,
    active_session_dir: ReadSignal<Option<String>>,
}
impl AppRouter {
    pub fn new(active_session_dir: ReadSignal<Option<String>>) -> Self {
        let (route, set_route) = signal(current_path());
        if let Some(window) = window() {
            let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                set_route.set(current_path());
                if let Some(session_dir) = active_session_dir.get_untracked() {
                    let _ = crate::api::persist_selected_session_dir(&session_dir);
                }
            });
            let _ = window
                .add_event_listener_with_callback("popstate", listener.as_ref().unchecked_ref());
            // Keep exactly one listener for the lifetime of the client owner.
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
        Self {
            route,
            set_route,
            active_session_dir,
        }
    }
    pub fn is_chat(self) -> bool {
        !matches!(
            self.route.get().as_str(),
            "/resume" | "/context" | "/settings"
        )
    }
    pub fn navigate(self, path: &str) {
        if self.route.get_untracked() == path {
            return;
        }
        if let Some(window) = window()
            && let Ok(history) = window.history()
        {
            let path = self
                .active_session_dir
                .get_untracked()
                .map(|session_dir| crate::api::session_path(path, &session_dir))
                .unwrap_or_else(|| path.to_owned());
            let _ = history.push_state_with_url(&JsValue::NULL, "", Some(&path));
        }
        self.set_route.set(path.to_owned());
    }
    pub fn click(self, event: MouseEvent, path: &'static str) {
        if event.ctrl_key()
            || event.meta_key()
            || event.shift_key()
            || event.alt_key()
            || event.button() != 0
        {
            return;
        }
        event.prevent_default();
        self.navigate(path);
    }
}

pub(crate) fn current_path() -> String {
    window()
        .and_then(|window| window.location().pathname().ok())
        .unwrap_or_else(|| "/".to_owned())
}
