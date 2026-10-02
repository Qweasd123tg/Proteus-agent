mod analysis;
mod api;
mod app;
mod architecture;
mod architecture_map;
mod architecture_model;
mod configs;
mod context_map;
mod icons;
mod session_report;
mod types;
mod ui_utils;
mod usage_details;
mod visibility;

use leptos::mount::mount_to_body;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(app::App);
}
