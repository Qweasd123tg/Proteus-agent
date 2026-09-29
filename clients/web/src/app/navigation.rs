use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{MouseEvent, window};

#[derive(Clone, Copy)]
pub(super) struct AppRouter {
    pub route: ReadSignal<String>,
    pub chat_visible: ReadSignal<bool>,
    set_route: WriteSignal<String>,
    active_session_dir: ReadSignal<Option<String>>,
}
impl AppRouter {
    pub fn new(active_session_dir: ReadSignal<Option<String>>) -> Self {
        let (route, set_route) = signal(current_path());
        let (chat_visible, set_chat_visible) = signal(current_path() != "/settings");
        #[cfg(not(target_arch = "wasm32"))]
        let _ = set_chat_visible;
        if let Some(window) = window() {
            let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                let path = current_path();
                set_route.set(path.clone());
                crate::components::workspace::reveal(if path == "/settings" {
                    "settings"
                } else {
                    "chat"
                });
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
        #[cfg(target_arch = "wasm32")]
        if let Some(window) = window() {
            let listener = Closure::<dyn FnMut(web_sys::CustomEvent)>::new(
                move |event: web_sys::CustomEvent| {
                    if event.type_() == "proteus-workspace-visibility" {
                        match event.detail().as_string().as_deref() {
                            Some("chat:shown") => set_chat_visible.set(true),
                            Some("chat:hidden") => set_chat_visible.set(false),
                            _ => (),
                        }
                        return;
                    }
                    let path = if event.detail().as_string().as_deref() == Some("settings") {
                        "/settings"
                    } else {
                        "/"
                    };
                    if route.get_untracked() == path {
                        return;
                    }
                    if let Some(w) = web_sys::window() {
                        let url = active_session_dir
                            .get_untracked()
                            .map(|s| crate::api::session_path(path, &s))
                            .unwrap_or_else(|| path.to_owned());
                        if let Ok(h) = w.history() {
                            let _ = h.replace_state_with_url(&JsValue::NULL, "", Some(&url));
                        }
                    }
                    set_route.set(path.to_owned());
                },
            );
            if let Some(document) = window.document() {
                let _ = document.add_event_listener_with_callback(
                    "proteus-workspace-route",
                    listener.as_ref().unchecked_ref(),
                );
                let _ = document.add_event_listener_with_callback(
                    "proteus-workspace-visibility",
                    listener.as_ref().unchecked_ref(),
                );
                let listener = StoredValue::new_local(listener);
                on_cleanup(move || {
                    listener.with_value(|f| {
                        let _ = document.remove_event_listener_with_callback(
                            "proteus-workspace-visibility",
                            f.as_ref().unchecked_ref(),
                        );
                        let _ = document.remove_event_listener_with_callback(
                            "proteus-workspace-route",
                            f.as_ref().unchecked_ref(),
                        );
                    })
                });
            }
        }
        Self {
            route,
            chat_visible,
            set_route,
            active_session_dir,
        }
    }
    pub fn is_chat(self) -> bool {
        !matches!(self.route.get().as_str(), "/settings")
    }
    pub fn navigate(self, path: &str) {
        if self.route.get_untracked() == path {
            crate::components::workspace::reveal(if path == "/settings" {
                "settings"
            } else {
                "chat"
            });
            return;
        }
        if let Some(window) = window()
            && let Ok(history) = window.history()
        {
            let path = self
                .active_session_dir
                .get_untracked()
                .or_else(crate::api::requested_session_dir)
                .map(|session_dir| crate::api::session_path(path, &session_dir))
                .unwrap_or_else(|| path.to_owned());
            let _ = history.push_state_with_url(&JsValue::NULL, "", Some(&path));
        }
        self.set_route.set(path.to_owned());
        crate::components::workspace::reveal(if path == "/settings" {
            "settings"
        } else {
            "chat"
        });
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
