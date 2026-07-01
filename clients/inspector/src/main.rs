mod api;
mod app;
mod architecture;
mod architecture_map;
mod architecture_model;
mod configs;
mod types;
mod ui_utils;

use leptos::mount::mount_to_body;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(app::App);
}
