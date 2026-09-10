mod actions;
mod api;
mod app;
mod app_keyboard;
mod app_resize;
mod app_sessions;
mod app_toasts;
mod chat_scroll;
mod components;
mod events;
mod markdown;
mod messages;
mod model_settings;
mod session;
mod tool_names;
mod transcript;
mod types;
mod ui_layout;
mod ui_preferences;
mod ui_utils;

use leptos::mount::mount_to_body;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(app::App);
}
